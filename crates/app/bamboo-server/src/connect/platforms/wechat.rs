//! 微信个人号平台适配器（腾讯官方 iLink Bot 协议，微信 8.0.70+ 内置支持）。
//!
//! 与 Telegram 适配器同构：纯 HTTP/JSON 长轮询——无公网 IP、无回调 URL、
//! 无 WebSocket， Bamboo 主动外连 `ilinkai.weixin.qq.com`。协议三要点：
//!
//! - **认证**：每个请求携带 `Authorization: Bearer <bot_token>`、
//!   `AuthorizationType: ilink_bot_token`，以及随机生成的 `X-WECHAT-UIN`
//!   （随机 uint32 的十进制字符串再 base64，防重放，每次请求重新生成）。
//! - **接收**：`POST /ilink/bot/getupdates` 长轮询（服务端最多挂起约 35 秒）。
//!   响应里的 `get_updates_buf` 是游标，下次请求必须原样回传；游标持久化到
//!   `state_dir/cursor.json`，否则重启后消息会重复投递。
//! - **发送**：`POST /ilink/bot/sendmessage`。请求体的 `msg.context_token`
//!   必须原样回传"触发这次回复的那条入站消息"携带的 context_token——
//!   机器人只能回复用户先发起的会话，无法主动发起。
//!
//! 会话过期（`ret == -14`）时自动进入扫码重登：拉取 `get_bot_qrcode` 把
//! 二维码 PNG 写到 `state_dir/login_qr.png` 并在日志输出路径，轮询
//! `get_qrcode_status` 直至用户扫码确认，拿到新 token 后恢复轮询。
//!
//! v1 范围（见 `docs/design/wechat-ilink-adapter-plan.md`）：仅私聊文本；
//! 媒体（CDN + AES-128-ECB）、输入中指示、群聊均推迟。

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use base64::Engine as _;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use super::super::platform::{
    Capabilities, Inbound, InboundMessage, MessageRef, OutboundMessage, Platform, PlatformError,
    PlatformResult, ReplyCtx,
};
use super::super::render::chunk_message;

/// iLink 网关官方域名（登录响应可能返回按 bot 区分的 `baseurl`，届时覆盖）。
const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
/// 服务端长轮询挂起时长（响应里的 `longpolling_timeout_ms` 为 35000）。
const LONG_POLL_TIMEOUT_SECS: u64 = 35;
/// HTTP 客户端超时在服务端挂起时长之上留的余量，避免客户端先掐断合法长轮询。
const CLIENT_POLL_MARGIN_SECS: u64 = 15;
/// 普通传输/解析失败后的重试退避。
const RETRY_BACKOFF: Duration = Duration::from_secs(5);
/// 会话过期（ret=-14）且扫码重登失败/超时后的再试间隔。
const SESSION_EXPIRED_BACKOFF: Duration = Duration::from_secs(300);
/// iLink 的"会话过期"错误码（cc-connect 实测：多为 token 失效，需重新扫码）。
const SESSION_EXPIRED_RET: i64 = -14;
/// 错误文本中携带的会话过期标记——`PlatformError` 是字符串错误，用包含匹配分流。
const SESSION_EXPIRED_MARKER: &str = "ret=-14";
/// 默认出站限流：每个会话 1 条/秒。微信未公布频率上限，先取保守值。
const DEFAULT_RATE_LIMIT_INTERVAL: Duration = Duration::from_secs(1);
/// 单条出站消息的字符上限。微信未公开此值，2000 为保守起步，真机校准后调整。
const WECHAT_MESSAGE_CHARS: usize = 2000;
/// `get_bot_qrcode` 的 `bot_type` 参数（协议文档示例值）。
const LOGIN_BOT_TYPE: &str = "3";
/// 等待用户扫码确认的总时长（对齐 cc-connect 的 480 秒默认值）。
const QR_LOGIN_TIMEOUT: Duration = Duration::from_secs(480);
/// 扫码状态轮询间隔（协议示例脚本为 1 秒一次）。
const QR_STATUS_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// 上报给网关的自身版本（`base_info.channel_version`，仅用于服务端观测）。
const CHANNEL_VERSION: &str = "bamboo-connect/0.1.0";
/// `msgs[].message_type == 1`：用户发来的入站消息。
const MSG_TYPE_INBOUND: i64 = 1;
/// 出站消息的 `message_type`（机器人发出）。
const MSG_TYPE_OUTBOUND: i64 = 2;
/// `message_state == 2`：FINISH（协议示例中的定值）。
const MSG_STATE_FINISH: i64 = 2;
/// `item_list[].type == 1`：文本条目。
const ITEM_TYPE_TEXT: i64 = 1;

/// 全仓库共享一个 `reqwest::Client`（对齐 telegram 适配器的 `http_client` 惯例，
/// 复用 workspace 锁定的 native-tls 栈，绝不另建第二个连接池）。
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// 生成一次性 `X-WECHAT-UIN`：随机 uint32 → 十进制字符串 → base64。
/// 协议把它用作防重放的一次性值，每次请求都必须重新生成。
fn random_wechat_uin() -> String {
    use rand::Rng;
    let value = rand::rng().next_u32();
    base64::engine::general_purpose::STANDARD.encode(value.to_string().as_bytes())
}

/// 每个会话的出站限流桶：阻塞（绝不丢弃）直到距上次发送至少 `min_interval`。
/// 在短临界区内原子地预定下一个可用时隙，然后在锁外睡眠——某个会话排队
/// 不会阻塞发往其他会话的消息。照抄 telegram 适配器的实现（含过期条目清扫）。
struct RateLimiter {
    next_allowed: AsyncMutex<HashMap<String, tokio::time::Instant>>,
    min_interval: Duration,
}

impl RateLimiter {
    fn new(min_interval: Duration) -> Self {
        Self {
            next_allowed: AsyncMutex::new(HashMap::new()),
            min_interval,
        }
    }

    async fn wait(&self, key: &str) {
        let now = tokio::time::Instant::now();
        let scheduled = {
            let mut guard = self.next_allowed.lock().await;
            let earliest = guard.get(key).copied().unwrap_or(now);
            let scheduled = earliest.max(now);
            guard.insert(key.to_string(), scheduled + self.min_interval);
            // 限制增长：清扫所有预定时刻已过期的条目，避免每个出现过的会话
            // 都在进程生命周期内占一条。被清扫键的下一次 `wait()` 经
            // `unwrap_or(now)` 回退为"立即"，行为不变。
            guard.retain(|_, next_allowed| *next_allowed > now);
            scheduled
        };
        if scheduled > now {
            tokio::time::sleep(scheduled - now).await;
        }
    }
}

// ---------------------------------------------------------------------------
// iLink 线格式（只声明本适配器用到的字段；serde 默认忽略未知字段）
// ---------------------------------------------------------------------------

/// `getupdates` 响应。
#[derive(Debug, serde::Deserialize)]
struct GetUpdatesResponse {
    #[serde(default)]
    ret: i64,
    #[serde(default)]
    errmsg: Option<String>,
    #[serde(default)]
    msgs: Vec<IlinkMessage>,
    /// 游标：下次请求必须回传。空/缺失时保持旧游标不变（协议未见空游标语义，
    /// 保守起见不推进）。
    #[serde(default)]
    get_updates_buf: Option<String>,
}

/// `msgs[]` 的一条消息。注意：`message_type == 2` 是机器人自己出站消息的
/// 回显——必须过滤，否则会形成自回复死循环。
#[derive(Debug, serde::Deserialize)]
struct IlinkMessage {
    #[serde(default)]
    from_user_id: Option<String>,
    #[serde(default)]
    message_type: i64,
    /// 上下文令牌：回复时必须原样回传；缺失的消息无法回复，直接丢弃。
    #[serde(default)]
    context_token: Option<String>,
    /// 网关分配的消息 id（优先作为去重键；缺失时退回合成哈希）。
    #[serde(default, rename = "message_id")]
    gateway_message_id: Option<i64>,
    /// 毫秒时间戳（cc-connect 的线格式字段名是 create_time_ms）。
    #[serde(default)]
    create_time_ms: Option<i64>,
    #[serde(default)]
    item_list: Vec<IlinkItem>,
}

#[derive(Debug, serde::Deserialize)]
struct IlinkItem {
    #[serde(default, rename = "type")]
    kind: Option<i64>,
    #[serde(default)]
    text_item: Option<IlinkTextItem>,
}

#[derive(Debug, serde::Deserialize)]
struct IlinkTextItem {
    #[serde(default)]
    text: Option<String>,
}

/// `sendmessage` 响应。注意错误有两条通道：`ret`（信封级）与 `errcode`
/// （业务级，cc-connect 的 `sessionExpiredErrcode = -14` 走这里）——两者都
/// 必须检查，否则会出现 ret=0 但消息实际被网关丢弃的静默失败。
#[derive(Debug, serde::Deserialize)]
struct SendResponse {
    #[serde(default)]
    ret: i64,
    #[serde(default)]
    errcode: i64,
    #[serde(default)]
    errmsg: Option<String>,
}

/// `get_bot_qrcode` 响应。
#[derive(Debug, serde::Deserialize)]
struct QrCodeResponse {
    #[serde(default)]
    ret: i64,
    #[serde(default)]
    errmsg: Option<String>,
    /// 二维码标识，用于轮询 `get_qrcode_status`。
    #[serde(default)]
    qrcode: Option<String>,
    /// 二维码图片内容（base64）。仅用于落盘供用户扫码，不是秘密。
    #[serde(default)]
    qrcode_img_content: Option<String>,
    /// 可在日志中展示的登录链接。
    #[serde(default)]
    url: Option<String>,
}

/// `get_qrcode_status` 响应。扫码确认（`status == "confirmed"`）时携带
/// `bot_token` 与按 bot 区分的 `baseurl`。
#[derive(Debug, serde::Deserialize)]
struct QrStatusResponse {
    #[serde(default)]
    ret: i64,
    #[serde(default)]
    errmsg: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    bot_token: Option<String>,
    #[serde(default)]
    baseurl: Option<String>,
}

/// 统一的 iLink 错误构造：错误文本同时携带 `ret` 与 `errcode`——
/// `start()` 靠 [`is_session_expired_text`]（ret=-14 或 errcode=-14）分流重登路径。
fn ret_error(operation: &str, ret: i64, errmsg: Option<String>) -> PlatformError {
    ret_error_full(operation, ret, 0, errmsg)
}

fn ret_error_full(operation: &str, ret: i64, errcode: i64, errmsg: Option<String>) -> PlatformError {
    let detail = errmsg.unwrap_or_else(|| "no errmsg".to_string());
    PlatformError::other(format!(
        "wechat {operation} failed with ret={ret} errcode={errcode}: {detail}"
    ))
}

/// 会话过期可能走 `ret` 或 `errcode` 两条通道（cc-connect 的
/// `sessionExpiredErrcode = -14`），两者都识别。
fn is_session_expired_text(text: &str) -> bool {
    text.contains("ret=-14") || text.contains("errcode=-14")
}

// ---------------------------------------------------------------------------
// 适配器
// ---------------------------------------------------------------------------

pub struct WechatPlatform {
    /// 网关地址：构造时为配置解析出的默认/自定义域；扫码重登成功后若返回
    /// 了按 bot 区分的 `baseurl` 则覆盖。
    base_url: RwLock<String>,
    /// Bearer token。正常来自 connect.json（加密管道解出）；扫码重登成功后
    /// 在内存中替换（进程重启即失效，日志会提示写入 connect.json 持久化）。
    token: RwLock<String>,
    /// 游标与登录二维码的落盘目录（`{data_dir}/connect_wechat/`）。
    state_dir: Option<PathBuf>,
    /// `get_updates_buf` 游标（内存权威副本；每次成功拉取后落盘）。
    cursor: AsyncMutex<Option<String>>,
    rate_limiter: RateLimiter,
}

impl WechatPlatform {
    /// 生产构造：官方网关 + 默认限流。
    pub fn new(token: String, base_url: String, state_dir: Option<PathBuf>) -> Self {
        Self::with_options(token, base_url, DEFAULT_RATE_LIMIT_INTERVAL, state_dir)
    }

    /// 测试/高级构造：可注入本地 HTTP 桩地址与极小的限流间隔。
    pub fn with_options(
        token: String,
        base_url: String,
        rate_limit_interval: Duration,
        state_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            base_url: RwLock::new(base_url),
            token: RwLock::new(token),
            state_dir,
            cursor: AsyncMutex::new(None),
            rate_limiter: RateLimiter::new(rate_limit_interval),
        }
    }

    fn base_url(&self) -> String {
        self.base_url
            .read()
            .expect("wechat base_url lock poisoned")
            .clone()
    }

    fn token(&self) -> String {
        self.token
            .read()
            .expect("wechat token lock poisoned")
            .clone()
    }

    /// 带 iLink 认证三头的请求构造器。`X-WECHAT-UIN` 每次请求重新生成
    /// （防重放一次性值）；token 为空时省略 `Authorization`（扫码端点在
    /// 取得 token 之前就要可调）。
    fn ilink_request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        let mut builder = http_client()
            .request(method, url)
            .header("AuthorizationType", "ilink_bot_token")
            .header("X-WECHAT-UIN", random_wechat_uin());
        let token = self.token();
        if !token.is_empty() {
            builder = builder.bearer_auth(token);
        }
        builder
    }

    /// 把 `reqwest::Error` 格式化为不含 bot token 的文本。token 虽然在
    /// `Authorization` 头而非 URL 里，但代理/网关错误可能回显目标信息，
    /// 照 telegram 适配器的双保险做法：先剥离 URL，再替换字面量。
    fn sanitize_error(&self, error: reqwest::Error) -> String {
        let text = error.without_url().to_string();
        let token = self.token();
        if token.is_empty() {
            text
        } else {
            text.replace(&token, "[REDACTED]")
        }
    }

    fn cursor_path(&self) -> Option<PathBuf> {
        self.state_dir.as_ref().map(|dir| dir.join("cursor.json"))
    }

    /// 把游标写入 `state_dir/cursor.json`。游标不是秘密；小文件、低频写，
    /// 直接整写即可（bridge 的原子写助手是它模块私有的）。
    fn persist_cursor(&self, cursor: &str) {
        let Some(path) = self.cursor_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let body = serde_json::json!({ "get_updates_buf": cursor }).to_string();
        if let Err(error) = std::fs::write(&path, body) {
            tracing::warn!(
                "connect: wechat failed to persist polling cursor ({}): {error}",
                path.display()
            );
        }
    }

    /// 启动时从磁盘恢复游标——这是重启后不重复消费消息的第一道防线
    /// （第二道是 bridge 的 `platform:message_id` 去重；协议没有时间戳字段，
    /// `sent_at` 的过期丢弃因此弱化）。
    async fn load_cursor(&self) {
        let Some(path) = self.cursor_path() else { return };
        let Ok(text) = std::fs::read_to_string(&path) else { return };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else { return };
        if let Some(buf) = value.get("get_updates_buf").and_then(|v| v.as_str()) {
            *self.cursor.lock().await = Some(buf.to_string());
        }
    }

    /// 一轮 `getupdates`：回传游标 → 校验 `ret` → 保存新游标（内存 + 磁盘）→
    /// 把入站消息映射为 bridge 事件。抽成独立方法便于 wiremock 单测确定性驱动。
    async fn poll_once(&self) -> PlatformResult<Vec<Inbound>> {
        let cursor = self.cursor.lock().await.clone().unwrap_or_default();
        let url = format!("{}/ilink/bot/getupdates", self.base_url());
        let body = serde_json::json!({
            "get_updates_buf": cursor,
            "base_info": { "channel_version": CHANNEL_VERSION },
        });

        let response = self
            .ilink_request(reqwest::Method::POST, &url)
            .timeout(Duration::from_secs(LONG_POLL_TIMEOUT_SECS + CLIENT_POLL_MARGIN_SECS))
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                PlatformError::other(format!(
                    "getupdates request failed: {}",
                    self.sanitize_error(error)
                ))
            })?;

        let parsed: GetUpdatesResponse = response.json().await.map_err(|error| {
            PlatformError::other(format!(
                "getupdates response parse failed: {}",
                self.sanitize_error(error)
            ))
        })?;

        if parsed.ret != 0 {
            return Err(ret_error("getupdates", parsed.ret, parsed.errmsg));
        }

        if let Some(new_cursor) = parsed.get_updates_buf.filter(|c| !c.is_empty()) {
            *self.cursor.lock().await = Some(new_cursor.clone());
            self.persist_cursor(&new_cursor);
        }

        Ok(parsed
            .msgs
            .iter()
            .enumerate()
            .filter_map(|(index, msg)| Self::to_inbound_message(msg, index))
            .map(Inbound::Message)
            .collect())
    }

    /// 把一条 iLink 消息映射为 bridge 的 [`InboundMessage`]。
    ///
    /// 返回 `None` 的消息（出站回显、无文本、无 `from_user_id`、无
    /// `context_token`）只是不转发——游标已在 `poll_once` 里推进，网关不会
    /// 重发它们（这点与 telegram 的 offset 语义一致）。
    ///
    /// `message_id` 是合成值：协议没有消息 id，用
    /// `from + context_token + 文本 + 批内序号` 的确定性哈希，保证同一条
    /// 消息在游标重放时哈希一致、bridge 去重键 `wechat:<hash>` 能命中。
    fn to_inbound_message(msg: &IlinkMessage, batch_index: usize) -> Option<InboundMessage> {
        if msg.message_type != MSG_TYPE_INBOUND {
            return None;
        }
        let from = msg
            .from_user_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())?;
        let text = extract_text(msg)?;
        let context_token = msg
            .context_token
            .clone()
            .filter(|value| !value.trim().is_empty())?;
        let sent_at = msg
            .create_time_ms
            .and_then(|millis| chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis))
            .unwrap_or_else(chrono::Utc::now);
        // 去重 id：优先用网关分配的 message_id；缺失时退回确定性哈希合成。
        let message_id = match msg.gateway_message_id {
            Some(id) => id.to_string(),
            None => synthesize_message_id(from, &context_token, &text, batch_index),
        };

        Some(InboundMessage {
            platform: "wechat".to_string(),
            // v1 仅私聊：会话与发送者都是对端用户。群聊（@chatroom）推迟，
            // 协议的群内发送者字段未公开。
            chat_id: from.to_string(),
            user_id: from.to_string(),
            message_id,
            sent_at,
            text,
            reply_ctx: ReplyCtx(serde_json::json!({
                "to_user_id": from,
                "context_token": context_token,
            })),
        })
    }

    /// 发送一条文本消息（`reply` 对每个分块调用一次）。线格式对齐 cc-connect
    /// 的 `sendMessageReq`：请求根级必须带 `base_info`，msg 必须带 `from_user_id`
    /// （bot 发送时为空串）与 `client_id`（客户端生成的消息 id）——缺这些字段
    /// 时网关可能返回 ret=0 却不投递消息（静默失败）。
    async fn send_message(&self, to_user_id: &str, context_token: &str, text: &str) -> PlatformResult<()> {
        let url = format!("{}/ilink/bot/sendmessage", self.base_url());
        let body = serde_json::json!({
            "msg": {
                "from_user_id": "",
                "to_user_id": to_user_id,
                "client_id": uuid::Uuid::new_v4().to_string(),
                "message_type": MSG_TYPE_OUTBOUND,
                "message_state": MSG_STATE_FINISH,
                "context_token": context_token,
                "item_list": [
                    { "type": ITEM_TYPE_TEXT, "text_item": { "text": text } }
                ],
            },
            "base_info": { "channel_version": CHANNEL_VERSION },
        });

        let response = self
            .ilink_request(reqwest::Method::POST, &url)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                PlatformError::other(format!(
                    "sendmessage request failed: {}",
                    self.sanitize_error(error)
                ))
            })?;

        let parsed: SendResponse = response.json().await.map_err(|error| {
            PlatformError::other(format!(
                "sendmessage response parse failed: {}",
                self.sanitize_error(error)
            ))
        })?;

        if parsed.ret != 0 || parsed.errcode != 0 {
            return Err(ret_error_full(
                "sendmessage",
                parsed.ret,
                parsed.errcode,
                parsed.errmsg,
            ));
        }
        Ok(())
    }

    /// 扫码登录/重登：拉取二维码 → PNG 落盘 + 日志输出链接 → 轮询扫码状态
    /// 直到 `confirmed`。成功后用新 token（及可选的新 `baseurl`）继续轮询；
    /// 超时返回错误，由调用方退避后重试整个流程。
    async fn qr_login(&self) -> PlatformResult<()> {
        let qr_url = format!("{}/ilink/bot/get_bot_qrcode", self.base_url());
        let response = self
            .ilink_request(reqwest::Method::GET, &qr_url)
            .query(&[("bot_type", LOGIN_BOT_TYPE)])
            .send()
            .await
            .map_err(|error| {
                PlatformError::other(format!(
                    "get_bot_qrcode request failed: {}",
                    self.sanitize_error(error)
                ))
            })?;
        let parsed: QrCodeResponse = response.json().await.map_err(|error| {
            PlatformError::other(format!(
                "get_bot_qrcode response parse failed: {}",
                self.sanitize_error(error)
            ))
        })?;
        if parsed.ret != 0 {
            return Err(ret_error("get_bot_qrcode", parsed.ret, parsed.errmsg));
        }

        // 二维码的交付形式（2026-10 实测网关返回 URL 形式，字段名有误导性）：
        // `qrcode_img_content` 为 http(s) 链接时，在浏览器打开该页面即可扫码
        // （手机微信内点开则直接弹确认）；为 base64 PNG 时落盘供直接扫码。
        // 两种内容都不是秘密。
        let image = parsed.qrcode_img_content.as_deref().unwrap_or_default();
        if image.starts_with("http://") || image.starts_with("https://") {
            tracing::warn!(
                "connect: wechat login page (open it in a browser and scan the QR with \
                 WeChat, or tap the link inside WeChat on your phone): {image}"
            );
        } else if !image.is_empty() {
            if let Some(dir) = &self.state_dir {
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(image) {
                    if !bytes.is_empty() {
                        let _ = std::fs::create_dir_all(dir);
                        let path = dir.join("login_qr.png");
                        match std::fs::write(&path, &bytes) {
                            Ok(()) => {
                                tracing::warn!(
                                    "connect: wechat login QR saved to {} — scan it with WeChat \
                                     to re-authenticate this bot",
                                    path.display()
                                );
                            }
                            Err(error) => {
                                tracing::warn!(
                                    "connect: wechat failed to save login QR to {}: {error}",
                                    path.display()
                                );
                            }
                        }
                    }
                }
            }
        }
        if let Some(link) = parsed.url.as_deref().filter(|value| !value.is_empty()) {
            tracing::warn!("connect: wechat login link: {link}");
        }

        let qrcode_id = parsed.qrcode.filter(|value| !value.is_empty()).ok_or_else(
            || PlatformError::other("get_bot_qrcode response missing qrcode id"),
        )?;

        let deadline = tokio::time::Instant::now() + QR_LOGIN_TIMEOUT;
        loop {
            tokio::time::sleep(QR_STATUS_POLL_INTERVAL).await;
            if tokio::time::Instant::now() >= deadline {
                return Err(PlatformError::other(
                    "wechat QR login timed out waiting for scan confirmation",
                ));
            }

            let status_url = format!("{}/ilink/bot/get_qrcode_status", self.base_url());
            let response = match self
                .ilink_request(reqwest::Method::GET, &status_url)
                .query(&[("qrcode", qrcode_id.as_str())])
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    // 状态轮询的瞬时失败不终止整个登录流程（截止时间兜底）。
                    tracing::warn!(
                        "connect: wechat get_qrcode_status failed, retrying: {}",
                        self.sanitize_error(error)
                    );
                    continue;
                }
            };
            let parsed: QrStatusResponse = match response.json().await {
                Ok(parsed) => parsed,
                Err(error) => {
                    tracing::warn!(
                        "connect: wechat get_qrcode_status parse failed, retrying: {}",
                        self.sanitize_error(error)
                    );
                    continue;
                }
            };
            if parsed.ret != 0 {
                tracing::warn!(
                    "connect: wechat get_qrcode_status returned ret={}, retrying: {}",
                    parsed.ret,
                    parsed.errmsg.unwrap_or_else(|| "no errmsg".to_string())
                );
                continue;
            }

            if parsed.status.as_deref() == Some("confirmed") {
                let token = parsed
                    .bot_token
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        PlatformError::other(
                            "get_qrcode_status confirmed but returned no bot_token",
                        )
                    })?;
                if let Some(baseurl) = parsed
                    .baseurl
                    .as_deref()
                    .filter(|value| value.starts_with("https://"))
                {
                    *self.base_url.write().expect("wechat base_url lock poisoned") =
                        baseurl.trim_end_matches('/').to_string();
                }
                *self.token.write().expect("wechat token lock poisoned") = token;
                tracing::warn!(
                    "connect: wechat QR login succeeded — the new bot_token lives in memory \
                     only; copy it into connect.json (platforms[type=wechat].token) so \
                     restarts reuse it"
                );
                return Ok(());
            }
        }
    }
}

/// 拼接一条消息里的全部文本条目（`item_list` 里可能有多段文本）。
/// 没有任何文本内容的消息（图片/语音/文件等，v1 不支持）返回 `None`。
fn extract_text(msg: &IlinkMessage) -> Option<String> {
    let parts: Vec<&str> = msg
        .item_list
        .iter()
        .filter(|item| item.kind == Some(ITEM_TYPE_TEXT))
        .filter_map(|item| item.text_item.as_ref())
        .filter_map(|text_item| text_item.text.as_deref())
        .filter(|text| !text.trim().is_empty())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

/// 合成确定性 message_id（见 [`WechatPlatform::to_inbound_message`] 的说明）。
fn synthesize_message_id(
    from_user_id: &str,
    context_token: &str,
    text: &str,
    batch_index: usize,
) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    from_user_id.hash(&mut hasher);
    context_token.hash(&mut hasher);
    text.hash(&mut hasher);
    batch_index.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[async_trait::async_trait]
impl Platform for WechatPlatform {
    fn name(&self) -> &str {
        "wechat"
    }

    /// 微信 v1 只有纯文本：无按钮（审批走编号文本回退）、无消息编辑
    /// （render 自动进入逐条发送的 legacy 模式）、无图片/文件。
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            buttons: false,
            edit_message: false,
            images: false,
            files: false,
        }
    }

    async fn start(&self, inbound: mpsc::Sender<Inbound>) -> PlatformResult<()> {
        self.load_cursor().await;

        loop {
            match self.poll_once().await {
                Ok(events) => {
                    for event in events {
                        if inbound.send(event).await.is_err() {
                            // 接收端已关闭：manager 正在关闭。
                            return Ok(());
                        }
                    }
                }
                Err(error) => {
                    if is_session_expired_text(&error.to_string()) {
                        // 会话过期（ret=-14）：进入扫码重登；失败则退避后重试
                        // 整个流程（二维码会重新挂出）。
                        tracing::warn!(
                            "connect: wechat session expired (ret=-14); starting QR re-login"
                        );
                        if let Err(login_error) = self.qr_login().await {
                            tracing::warn!(
                                "connect: wechat QR re-login failed: {login_error}; retrying \
                                 in {SESSION_EXPIRED_BACKOFF:?}"
                            );
                            tokio::time::sleep(SESSION_EXPIRED_BACKOFF).await;
                        }
                    } else {
                        tracing::warn!("connect: wechat getupdates failed, retrying: {error}");
                        tokio::time::sleep(RETRY_BACKOFF).await;
                    }
                }
            }
        }
    }

    async fn reply(&self, ctx: &ReplyCtx, msg: OutboundMessage) -> PlatformResult<MessageRef> {
        let to_user_id = ctx
            .0
            .get("to_user_id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| PlatformError::other("reply_ctx is missing to_user_id"))?
            .to_string();
        let context_token = ctx
            .0
            .get("context_token")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| PlatformError::other("reply_ctx is missing context_token"))?
            .to_string();

        // `msg.buttons` 被忽略：capabilities 未声明按钮，bridge/approvals 不会
        // 传入（编号文本列表才是微信侧的审批呈现方式）。
        for chunk in chunk_message(&msg.text, WECHAT_MESSAGE_CHARS) {
            self.rate_limiter.wait(&to_user_id).await;
            self.send_message(&to_user_id, &context_token, &chunk).await?;
        }

        Ok(MessageRef(serde_json::json!({ "to_user_id": to_user_id })))
    }

    async fn edit(&self, _msg_ref: &MessageRef, _new: OutboundMessage) -> PlatformResult<()> {
        // capabilities.edit_message = false：render 层不会调用；留一个明确
        // 的错误而不是静默成功，防止未来能力声明变化时出现隐性假成功。
        Err(PlatformError::other(
            "wechat adapter does not support message editing",
        ))
    }

    async fn stop(&self) -> PlatformResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TOKEN: &str = "ilink-test-token";

    fn platform_with_stub(base_url: String) -> WechatPlatform {
        WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            base_url,
            Duration::from_millis(50),
            None,
        )
    }

    async fn wait_for_requests(
        server: &wiremock::MockServer,
        expected: usize,
    ) -> Vec<wiremock::Request> {
        for _ in 0..100 {
            if let Some(requests) = server.received_requests().await {
                if requests.len() >= expected {
                    return requests;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        server.received_requests().await.unwrap_or_default()
    }

    fn body_json(request: &wiremock::Request) -> serde_json::Value {
        serde_json::from_slice(request.body.as_slice()).expect("request body is valid JSON")
    }

    async fn mount_getupdates(server: &wiremock::MockServer, response: serde_json::Value) {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/ilink/bot/getupdates"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(response))
            .mount(server)
            .await;
    }

    fn inbound_msg(text: &str) -> serde_json::Value {
        serde_json::json!({
            "from_user_id": "wxid_user@im.wechat",
            "to_user_id": "bot@im.bot",
            "message_type": 1,
            "message_state": 2,
            "context_token": "CTX-TOKEN-1",
            "item_list": [ { "type": 1, "text_item": { "text": text } } ]
        })
    }

    #[tokio::test]
    async fn poll_once_maps_inbound_text_with_reply_ctx() {
        let server = wiremock::MockServer::start().await;
        mount_getupdates(
            &server,
            serde_json::json!({
                "ret": 0,
                "msgs": [ inbound_msg("你好， Bamboo") ],
                "get_updates_buf": "CURSOR-1"
            }),
        )
            .await;

        let platform = platform_with_stub(server.uri());
        let events = platform.poll_once().await.expect("poll_once succeeds");

        assert_eq!(events.len(), 1);
        match &events[0] {
            Inbound::Message(message) => {
                assert_eq!(message.platform, "wechat");
                assert_eq!(message.chat_id, "wxid_user@im.wechat");
                assert_eq!(message.user_id, "wxid_user@im.wechat");
                assert_eq!(message.text, "你好， Bamboo");
                assert!(!message.message_id.is_empty());
                // 回复路由依赖的不透明上下文：to_user_id + 原样回传的
                // context_token。
                assert_eq!(
                    message.reply_ctx.0.get("to_user_id").and_then(|v| v.as_str()),
                    Some("wxid_user@im.wechat")
                );
                assert_eq!(
                    message.reply_ctx.0.get("context_token").and_then(|v| v.as_str()),
                    Some("CTX-TOKEN-1")
                );
            }
            Inbound::Callback(_) => panic!("expected a message event"),
        }

        // 游标已推进。
        assert_eq!(
            platform.cursor.lock().await.as_deref(),
            Some("CURSOR-1")
        );
    }

    /// `message_type == 2` 是机器人出站回显——不过滤就会自回复死循环；
    /// 非文本条目（图片等）与缺 context_token 的消息也一并跳过。
    #[tokio::test]
    async fn poll_once_drops_outbound_echoes_and_non_text_items() {
        let server = wiremock::MockServer::start().await;
        mount_getupdates(
            &server,
            serde_json::json!({
                "ret": 0,
                "msgs": [
                    { "from_user_id": "bot@im.bot", "message_type": 2,
                      "context_token": "CTX", "item_list": [ { "type": 1, "text_item": { "text": "自己的回显" } } ] },
                    { "from_user_id": "wxid_user@im.wechat", "message_type": 1,
                      "context_token": "CTX", "item_list": [ { "type": 2 } ] },
                    { "from_user_id": "wxid_user@im.wechat", "message_type": 1,
                      "item_list": [ { "type": 1, "text_item": { "text": "没有 context_token" } } ] }
                ],
                "get_updates_buf": "CURSOR-2"
            }),
        )
            .await;

        let platform = platform_with_stub(server.uri());
        let events = platform.poll_once().await.expect("poll_once succeeds");
        assert!(events.is_empty(), "all three messages must be dropped");
    }

    #[tokio::test]
    async fn poll_once_echoes_the_persisted_cursor_on_the_next_request() {
        let server = wiremock::MockServer::start().await;
        mount_getupdates(
            &server,
            serde_json::json!({ "ret": 0, "msgs": [], "get_updates_buf": "CURSOR-A" }),
        )
            .await;

        let platform = platform_with_stub(server.uri());
        platform.poll_once().await.unwrap();
        platform.poll_once().await.unwrap();

        let requests = wait_for_requests(&server, 2).await;
        let first = body_json(&requests[0]);
        let second = body_json(&requests[1]);
        assert_eq!(first.get("get_updates_buf").and_then(|v| v.as_str()), Some(""));
        assert_eq!(
            second.get("get_updates_buf").and_then(|v| v.as_str()),
            Some("CURSOR-A")
        );
    }

    /// 游标落盘/恢复：同一 state_dir 的新实例第一次请求就回传旧游标——
    /// 重启后不重复消费消息的关键。
    #[tokio::test]
    async fn cursor_survives_a_restart_via_state_dir() {
        let server = wiremock::MockServer::start().await;
        mount_getupdates(
            &server,
            serde_json::json!({ "ret": 0, "msgs": [], "get_updates_buf": "CURSOR-PERSISTED" }),
        )
            .await;

        let state_dir = std::env::temp_dir().join(format!(
            "bamboo-wechat-cursor-test-{}",
            uuid::Uuid::new_v4()
        ));
        let first = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            server.uri(),
            Duration::from_millis(50),
            Some(state_dir.clone()),
        );
        first.poll_once().await.unwrap();

        let restarted = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            server.uri(),
            Duration::from_millis(50),
            Some(state_dir),
        );
        restarted.load_cursor().await;
        restarted.poll_once().await.unwrap();

        let requests = wait_for_requests(&server, 2).await;
        let second = body_json(&requests[1]);
        assert_eq!(
            second.get("get_updates_buf").and_then(|v| v.as_str()),
            Some("CURSOR-PERSISTED")
        );

        let _ = std::fs::remove_dir_all(restarted.state_dir.clone().unwrap_or_default());
    }

    #[tokio::test]
    async fn reply_posts_context_token_and_chunks_long_text() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/ilink/bot/sendmessage"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ret": 0
                })),
            )
            .mount(&server)
            .await;

        let platform = platform_with_stub(server.uri());
        let ctx = ReplyCtx(serde_json::json!({
            "to_user_id": "wxid_user@im.wechat",
            "context_token": "CTX-REPLY-1"
        }));
        // 2500 字符 → 2000 + 500 两块。
        let long_text = "长".repeat(2500);
        platform
            .reply(&ctx, OutboundMessage::text(long_text))
            .await
            .expect("reply succeeds");

        let requests = wait_for_requests(&server, 2).await;
        for request in &requests {
            let body = body_json(request);
            // 请求根级必须带 base_info（缺它网关可能静默丢弃消息）。
            assert_eq!(
                body.get("base_info")
                    .and_then(|v| v.get("channel_version"))
                    .and_then(|v| v.as_str()),
                Some(CHANNEL_VERSION)
            );
            let msg = body.get("msg").expect("body has msg");
            assert_eq!(
                msg.get("to_user_id").and_then(|v| v.as_str()),
                Some("wxid_user@im.wechat")
            );
            // bot 发送时 from_user_id 为空串、client_id 为客户端生成的 id。
            assert_eq!(msg.get("from_user_id").and_then(|v| v.as_str()), Some(""));
            assert!(
                msg.get("client_id")
                    .and_then(|v| v.as_str())
                    .is_some_and(|v| !v.is_empty())
            );
            assert_eq!(
                msg.get("context_token").and_then(|v| v.as_str()),
                Some("CTX-REPLY-1")
            );
            assert_eq!(msg.get("message_type").and_then(|v| v.as_i64()), Some(2));
            assert_eq!(msg.get("message_state").and_then(|v| v.as_i64()), Some(2));
        }
        let text_of = |request: &wiremock::Request| {
            body_json(request)
                .pointer("/msg/item_list/0/text_item/text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .chars()
                .count()
        };
        assert_eq!(text_of(&requests[0]), 2000);
        assert_eq!(text_of(&requests[1]), 500);
    }

    #[tokio::test]
    async fn session_expired_error_carries_the_ret_code() {
        let server = wiremock::MockServer::start().await;
        mount_getupdates(
            &server,
            serde_json::json!({ "ret": -14, "errmsg": "session expired" }),
        )
            .await;

        let platform = platform_with_stub(server.uri());
        let error = platform.poll_once().await.expect_err("ret=-14 must fail");
        assert!(error.to_string().contains(SESSION_EXPIRED_MARKER));
    }

    /// ret=0 但 errcode!=0 是真实失败（cc-connect 的会话过期就走 errcode）——
    /// 适配器绝不能当成功吞掉。
    #[tokio::test]
    async fn sendmessage_errcode_failure_is_an_error_even_with_ret_zero() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/ilink/bot/sendmessage"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ret": 0,
                    "errcode": -14,
                    "errmsg": "session expired"
                })),
            )
            .mount(&server)
            .await;

        let platform = platform_with_stub(server.uri());
        let ctx = ReplyCtx(serde_json::json!({
            "to_user_id": "wxid_user@im.wechat",
            "context_token": "CTX"
        }));
        let error = platform
            .reply(&ctx, OutboundMessage::text("你好"))
            .await
            .expect_err("errcode=-14 must fail");
        let text = error.to_string();
        assert!(text.contains("errcode=-14"), "error was: {text}");
        assert!(is_session_expired_text(&text));
    }

    /// 传输失败绝不能把 bot token 泄进错误文本（对齐 telegram 的
    /// `transport_errors_never_leak_the_bot_token`）。
    #[tokio::test]
    async fn transport_errors_never_leak_the_bot_token() {
        // 占一个端口然后立即释放 → 连接必被拒绝，制造纯传输错误。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let platform = platform_with_stub(format!("http://127.0.0.1:{port}"));
        let error = platform.poll_once().await.expect_err("dead port must fail");
        let text = error.to_string();
        assert!(!text.contains(TEST_TOKEN), "error leaked the token: {text}");
    }

    /// -14 触发的扫码重登：桩掉二维码与扫码状态端点，验证 token/base_url
    /// 被新值替换。
    #[tokio::test]
    async fn qr_login_obtains_a_fresh_token_and_baseurl() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/ilink/bot/get_bot_qrcode"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ret": 0,
                    "qrcode": "QR-123",
                    // 实测网关把登录链接放在这个字段（名字有误导性），`url` 为空。
                    "qrcode_img_content":
                        "https://liteapp.weixin.qq.com/q/7GiQu1?qrcode=QR-123&bot_type=3",
                    "url": ""
                })),
            )
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/ilink/bot/get_qrcode_status"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ret": 0,
                    "status": "confirmed",
                    "bot_token": "fresh-ilink-token",
                    // 生产代码只接受 https:// 的 baseurl 覆盖（本地桩是 http://，
                    // 所以这里用一个独立的 https 地址验证覆盖生效）。
                    "baseurl": "https://ilink-bot-specific.example.com/"
                })),
            )
            .mount(&server)
            .await;

        let state_dir = std::env::temp_dir().join(format!(
            "bamboo-wechat-qr-test-{}",
            uuid::Uuid::new_v4()
        ));
        let platform = WechatPlatform::with_options(
            String::new(),
            server.uri(),
            Duration::from_millis(50),
            Some(state_dir.clone()),
        );

        platform.qr_login().await.expect("qr_login succeeds");

        assert_eq!(platform.token(), "fresh-ilink-token");
        assert_eq!(platform.base_url(), "https://ilink-bot-specific.example.com");

        let _ = std::fs::remove_dir_all(state_dir);
    }

    /// 出站限流：同一会话连续两条消息必须间隔 `min_interval`（用真实秒会拖慢
    /// 测试，这里用 150ms 间隔验证"第二条被推迟"而不是"立即发出"）。
    #[tokio::test]
    async fn replies_to_one_chat_are_rate_limited() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/ilink/bot/sendmessage"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ret": 0
                })),
            )
            .mount(&server)
            .await;

        let platform = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            server.uri(),
            Duration::from_millis(150),
            None,
        );
        let ctx = ReplyCtx(serde_json::json!({
            "to_user_id": "wxid_user@im.wechat",
            "context_token": "CTX"
        }));

        let started = std::time::Instant::now();
        platform
            .reply(&ctx, OutboundMessage::text("第一条"))
            .await
            .unwrap();
        platform
            .reply(&ctx, OutboundMessage::text("第二条"))
            .await
            .unwrap();
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(150),
            "second reply must be delayed by the per-chat rate limit"
        );
    }
}
