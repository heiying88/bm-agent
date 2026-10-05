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
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use base64::Engine as _;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

use super::super::platform::{
    Capabilities, Inbound, InboundMessage, MessageRef, OutboundMessage, Platform, PlatformError,
    PlatformResult, ReplyCtx,
};
use super::super::render::chunk_message;
use super::wechat_voice::{self, VoiceConfig};

/// iLink 网关官方域名（登录响应可能返回按 bot 区分的 `baseurl`，届时覆盖）。
const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
/// 媒体 CDN 根地址（入站图片/文件的下载源）。
const DEFAULT_CDN_BASE_URL: &str = "https://novac2c.cdn.weixin.qq.com/c2c";
/// 单个媒体的最大字节数（对齐 cc-connect 的 100 MB 上限）。
const MAX_MEDIA_BYTES: usize = 100 * 1024 * 1024;
/// 媒体下载超时。
const MEDIA_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
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
/// `item_list[].type == 2`：图片条目（CDN 加密媒体）。
const ITEM_TYPE_IMAGE: i64 = 2;
/// `item_list[].type == 3`：语音条目（含微信 ASR 转写文本）。
const ITEM_TYPE_VOICE: i64 = 3;
/// `item_list[].type == 4`：文件条目（CDN 加密媒体）。
const ITEM_TYPE_FILE: i64 = 4;
/// `item_list[].type == 5`：视频条目（CDN 加密媒体）。
const ITEM_TYPE_VIDEO: i64 = 5;
/// `getuploadurl` 的 `media_type`：图片。
const UPLOAD_MEDIA_IMAGE: i64 = 1;
/// `getuploadurl` 的 `media_type`：视频。
const UPLOAD_MEDIA_VIDEO: i64 = 2;
/// `getuploadurl` 的 `media_type`：文件。
const UPLOAD_MEDIA_FILE: i64 = 3;
/// `getuploadurl` 的 `media_type`：语音。**注意**：出站语音上传实际走
/// [`UPLOAD_MEDIA_FILE`]——官方包定义了 VOICE=4 但从未使用（无出站语音
/// 实现），真机验证 VOICE=4 上传 + sendmessage ret=0 但微信静默不显示；
/// cc-connect 的真机可用实现用 FILE=3 上传语音（"voice uses same CDN
/// upload mechanism"），此处保留枚举仅作协议文档。
const UPLOAD_MEDIA_VOICE: i64 = 4;
/// `voice_item.encode_type` 的兜底值。两份资料互相矛盾：官方包
/// `@tencent-weixin/openclaw-weixin` 注释 1=pcm…6=silk；cc-connect 注释
/// 0=AMR、1=SILK（其真机可用实现发 AMR+0）。真机实测 4 与 6 都被静默丢弃。
/// 因此出站值按优先级解析：环境变量 `BAMBOO_WECHAT_VOICE_ENCODE_TYPE` >
/// 入站观测值（微信自己的语音条带的 encode_type 即本线格式下 SILK 的
/// 真实值，自动校准）> 本兜底常量。
const VOICE_ENCODE_TYPE_SILK: i64 = 6;
/// 出站附件标记：回复文本中单独一行的 `[SEND_FILE: <绝对路径>]` 由本适配器
/// 解析并投递，标记行不展示给用户（bridge 在会话首条消息注入约定说明）。
const SEND_FILE_MARKER: &str = "[SEND_FILE: ";
/// 出站语音标记：单独一行的 `[SEND_VOICE: <要念的文本>]` 由本适配器解析并
/// 合成语音条投递（用户明确要求朗读时模型才使用；标记行不展示）。
const SEND_VOICE_MARKER: &str = "[SEND_VOICE: ";

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

// ---------------------------------------------------------------------------
// 入站媒体：CDN 下载 + AES-128-ECB 解密（对齐 cc-connect 的 cdn.go 实现）
// ---------------------------------------------------------------------------

/// 归一化媒体密钥：图片走 32 位 hex 字段；否则用 base64 的 `aes_key`
/// （解码后为 16 字节密钥，或 32 字符的 hex ASCII 字符串再解一次）。
fn normalize_media_key(hex_field: Option<&str>, b64_field: Option<&str>) -> Option<[u8; 16]> {
    if let Some(hex) = hex_field
        .map(str::trim)
        .filter(|value| value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        let mut key = [0u8; 16];
        for (index, byte) in key.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
        }
        return Some(key);
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64_field?.trim().as_bytes())
        .ok()?;
    match decoded.len() {
        16 => Some(decoded[..].try_into().expect("16 bytes")),
        32 => {
            // 32 字节且全是 hex 字符：实为 hex 字符串（cc-connect 的同款兼容）。
            let text = std::str::from_utf8(&decoded).ok()?;
            if text.bytes().all(|b| b.is_ascii_hexdigit()) {
                let mut key = [0u8; 16];
                for (index, byte) in key.iter_mut().enumerate() {
                    *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
                }
                Some(key)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// AES-128-ECB 解密（无 IV），逐块解密后剥 PKCS#7 填充。密文长度非 16 的
/// 整数倍视为损坏返回 `None`。
fn decrypt_aes_128_ecb(key: &[u8; 16], ciphertext: &[u8]) -> Option<Vec<u8>> {
    use aes_gcm::aes::cipher::common::Block;
    use aes_gcm::aes::cipher::{BlockCipherDecrypt, KeyInit};

    if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
        return None;
    }
    let cipher = aes_gcm::aes::Aes128::new(Block::<aes_gcm::aes::Aes128>::from_slice(key));
    let mut plain = ciphertext.to_vec();
    for chunk in plain.chunks_exact_mut(16) {
        cipher.decrypt_block(Block::<aes_gcm::aes::Aes128>::from_mut_slice(chunk));
    }
    pkcs7_unpad(&plain)
}

/// PKCS#7 去填充（块大小 16）：填充长度必须在 1..=16 且所有填充字节一致。
fn pkcs7_unpad(data: &[u8]) -> Option<Vec<u8>> {
    const BLOCK: usize = 16;
    if data.len() < BLOCK || data.len() % BLOCK != 0 {
        return None;
    }
    let pad = *data.last()? as usize;
    if pad == 0 || pad > BLOCK {
        return None;
    }
    if !data[data.len() - pad..].iter().all(|byte| *byte as usize == pad) {
        return None;
    }
    Some(data[..data.len() - pad].to_vec())
}

/// PKCS#7 填充（块大小 16）——出站上传前的加密预处理。
fn pkcs7_pad(data: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 16;
    let pad = BLOCK - data.len() % BLOCK;
    let mut out = data.to_vec();
    out.extend(std::iter::repeat(pad as u8).take(pad));
    out
}

/// AES-128-ECB 加密（无 IV）+ PKCS#7 填充——出站媒体上传用（对齐
/// cc-connect 的 `encryptAESECB`）。
fn encrypt_aes_128_ecb(key: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
    use aes_gcm::aes::cipher::common::Block;
    use aes_gcm::aes::cipher::{BlockCipherEncrypt, KeyInit};

    let cipher = aes_gcm::aes::Aes128::new(Block::<aes_gcm::aes::Aes128>::from_slice(key));
    let mut out = pkcs7_pad(plaintext);
    for chunk in out.chunks_exact_mut(16) {
        cipher.encrypt_block(Block::<aes_gcm::aes::Aes128>::from_mut_slice(chunk));
    }
    out
}

/// 出站媒体密钥的 API 形态：`base64(hex_string)`（对齐 cc-connect 的
/// `formatAesKeyForAPI`——sendmessage 里 `media.aes_key` 用这个格式）。
fn format_media_key_for_api(key: &[u8; 16]) -> String {
    let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    base64::engine::general_purpose::STANDARD.encode(hex.as_bytes())
}

/// 随机 16 字节的十六进制串（出站 filekey / aeskey 字段用）。
fn random_hex_16() -> String {
    let bytes: [u8; 16] = rand::random();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 从回复文本中提取 `[SEND_FILE: <绝对路径>]` 标记行：返回剥掉标记后的
/// 展示文本与待投递文件路径列表（保持出现顺序）。标记行必须是整行。
fn extract_send_file_markers(text: &str) -> (String, Vec<PathBuf>) {
    let mut files = Vec::new();
    let mut kept: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(SEND_FILE_MARKER) {
            if let Some(path) = rest.strip_suffix(']') {
                let path = path.trim();
                if !path.is_empty() {
                    files.push(PathBuf::from(path));
                    continue;
                }
            }
        }
        kept.push(line);
    }
    let visible = kept.join("\n").trim().to_string();
    (visible, files)
}

/// 剥离 `[SEND_VOICE: <要念的文本>]` 标记行，返回（可见文本，语音文本
/// 列表）。与 [`extract_send_file_markers`] 同一款解析，标记行不展示。
fn extract_send_voice_markers(text: &str) -> (String, Vec<String>) {
    let mut voices = Vec::new();
    let mut kept: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(SEND_VOICE_MARKER) {
            if let Some(voice_text) = rest.strip_suffix(']') {
                let voice_text = voice_text.trim();
                if !voice_text.is_empty() {
                    voices.push(voice_text.to_string());
                    continue;
                }
            }
        }
        kept.push(line);
    }
    let visible = kept.join("\n").trim().to_string();
    (visible, voices)
}

/// 魔数嗅探图片扩展名（对齐 cc-connect 的 detectImageMime）。
fn sniff_image_ext(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        "png"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "gif"
    } else if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP" {
        "webp"
    } else {
        "jpg"
    }
}

/// RFC 3986 查询参数编码：除未保留字符（`A-Za-z0-9-_.~`）外全部百分号编码。
fn percent_encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
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
    /// 语音条：微信 ASR 转写文本在 `text` 字段里——有转写时无需下载音频。
    #[serde(default)]
    voice_item: Option<IlinkVoiceItem>,
    #[serde(default)]
    image_item: Option<IlinkImageItem>,
    #[serde(default)]
    file_item: Option<IlinkFileItem>,
    #[serde(default)]
    video_item: Option<IlinkVideoItem>,
}

#[derive(Debug, serde::Deserialize)]
struct IlinkTextItem {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct IlinkVoiceItem {
    /// 微信自带的语音转写文本。
    #[serde(default)]
    text: Option<String>,
    /// 载荷编码类型。微信自己发出的语音条（SILK）带的就是本线格式下
    /// SILK 的真实取值——两份社区资料（官方包注释 6=silk 与 cc-connect
    /// 注释 1=SILK）矛盾，入站观测值用于出站自动校准。
    #[serde(default)]
    encode_type: Option<i64>,
    /// CDN 媒体引用（入站语音条也带；此前只取转写文本未解析）。
    #[serde(default)]
    media: Option<IlinkCdnMedia>,
    /// 以下字段仅为出站对齐取证保留（真机观测微信实际下发哪些字段）。
    #[serde(default)]
    playtime: Option<i64>,
    #[serde(default)]
    sample_rate: Option<i64>,
    #[serde(default)]
    bits_per_sample: Option<i64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct IlinkImageItem {
    #[serde(default)]
    media: Option<IlinkCdnMedia>,
    /// 图片特有的 32 位十六进制 AES 密钥（优先于 media.aes_key）。
    #[serde(default, rename = "aeskey")]
    aes_key_hex: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct IlinkFileItem {
    #[serde(default)]
    media: Option<IlinkCdnMedia>,
    /// 原始文件名（入站展示用）。
    #[serde(default)]
    file_name: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct IlinkVideoItem {
    #[serde(default)]
    media: Option<IlinkCdnMedia>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct IlinkCdnMedia {
    /// CDN 下载凭据（拼进 `/download?encrypted_query_param=...`）。
    #[serde(default)]
    encrypt_query_param: Option<String>,
    /// base64 的 AES-128 密钥（或 32 位 hex 的 ASCII 字符串）。
    #[serde(default)]
    aes_key: Option<String>,
    /// 加密打包形态（0=只加密 fileid，1=含缩略图等打包信息）。取证用。
    #[serde(default)]
    encrypt_type: Option<i64>,
}

/// 探针捕获的入站语音（诊断回显用）：微信自己的语音条 media 引用与
/// 字段原样保存，回显时一字不改地发回去。
#[derive(Debug, Clone)]
struct CapturedVoiceMedia {
    media: serde_json::Value,
    encode_type: Option<i64>,
    playtime: Option<i64>,
    sample_rate: Option<i64>,
    bits_per_sample: Option<i64>,
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

/// `getuploadurl` 响应：新版返回完整的 `upload_full_url`，旧版返回
/// `upload_param`（拼进 `{cdn}/upload?encrypted_query_param=...&filekey=...`）。
#[derive(Debug, serde::Deserialize)]
struct GetUploadUrlResponse {
    #[serde(default)]
    ret: i64,
    #[serde(default)]
    errcode: i64,
    #[serde(default)]
    errmsg: Option<String>,
    #[serde(default)]
    upload_param: Option<String>,
    #[serde(default)]
    upload_full_url: Option<String>,
}

/// 是否为可内联展示的位图（出站时按 image_item 发送）。与
/// [`sniff_image_ext`] 同一套魔数。
fn is_raster_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP")
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

// ---------------------------------------------------------------------------
// 扫码登录轻量客户端（设置页扫码配置流程与平台 qr_login 共用）
// ---------------------------------------------------------------------------

/// 默认 iLink 网关地址（设置页扫码流程无平台实例，直接用默认网关）。
pub(crate) fn ilink_default_base_url() -> &'static str {
    DEFAULT_BASE_URL
}

/// `get_bot_qrcode` 的结果（`image_kind` 区分登录页链接与内联 PNG）。
pub(crate) struct WechatLoginQr {
    pub qrcode_id: String,
    /// `"url"`（http 登录页链接）| `"png_base64"`（内联图片）| `""`（无图）。
    pub image_kind: &'static str,
    pub image_content: String,
    pub url: Option<String>,
}

/// `get_qrcode_status` 的结果（`token` 仅在 `confirmed` 时存在）。
pub(crate) struct WechatLoginStatus {
    pub status: String,
    pub token: Option<String>,
    pub baseurl: Option<String>,
}

/// 拉取登录二维码（免鉴权端点；`X-WECHAT-UIN` 为防重放一次性值）。
pub(crate) async fn fetch_login_qrcode(base_url: &str) -> Result<WechatLoginQr, String> {
    let url = format!("{base_url}/ilink/bot/get_bot_qrcode");
    let response = http_client()
        .request(reqwest::Method::GET, &url)
        .header("AuthorizationType", "ilink_bot_token")
        .header("X-WECHAT-UIN", random_wechat_uin())
        .query(&[("bot_type", LOGIN_BOT_TYPE)])
        .send()
        .await
        .map_err(|error| format!("get_bot_qrcode request failed: {}", error.without_url()))?;
    let parsed: QrCodeResponse = response
        .json()
        .await
        .map_err(|error| format!("get_bot_qrcode response parse failed: {error}"))?;
    if parsed.ret != 0 {
        return Err(format!(
            "get_bot_qrcode failed with ret={}: {}",
            parsed.ret,
            parsed.errmsg.unwrap_or_else(|| "no errmsg".to_string())
        ));
    }
    let qrcode_id = parsed
        .qrcode
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "get_bot_qrcode returned no qrcode id".to_string())?;
    let image_content = parsed.qrcode_img_content.unwrap_or_default();
    let image_kind = if image_content.starts_with("http://") || image_content.starts_with("https://")
    {
        "url"
    } else if !image_content.is_empty()
    {
        "png_base64"
    } else {
        ""
    };
    Ok(WechatLoginQr {
        qrcode_id,
        image_kind,
        image_content,
        url: parsed.url.filter(|value| !value.is_empty()),
    })
}

/// 查询扫码状态一次（`confirmed` 时携带新 bot_token；token 只在服务器侧
/// 使用，绝不透传给浏览器明文展示以外的路径）。
pub(crate) async fn poll_login_qrcode(
    base_url: &str,
    qrcode_id: &str,
) -> Result<WechatLoginStatus, String> {
    let url = format!("{base_url}/ilink/bot/get_qrcode_status");
    let response = http_client()
        .request(reqwest::Method::GET, &url)
        .header("AuthorizationType", "ilink_bot_token")
        .header("X-WECHAT-UIN", random_wechat_uin())
        .query(&[("qrcode", qrcode_id)])
        .send()
        .await
        .map_err(|error| format!("get_qrcode_status request failed: {}", error.without_url()))?;
    let parsed: QrStatusResponse = response
        .json()
        .await
        .map_err(|error| format!("get_qrcode_status response parse failed: {error}"))?;
    if parsed.ret != 0 {
        return Err(format!(
            "get_qrcode_status failed with ret={}: {}",
            parsed.ret,
            parsed.errmsg.unwrap_or_else(|| "no errmsg".to_string())
        ));
    }
    Ok(WechatLoginStatus {
        status: parsed.status.unwrap_or_else(|| "unknown".to_string()),
        token: parsed
            .bot_token
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty()),
        baseurl: parsed
            .baseurl
            .filter(|value| value.starts_with("https://"))
            .map(|value| value.trim_end_matches('/').to_string()),
    })
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
    /// 媒体 CDN 根地址（生产为官方域名；测试注入本地桩）。
    cdn_base_url: String,
    /// Bearer token。正常来自 connect.json（加密管道解出）；扫码重登成功后
    /// 在内存中替换（进程重启即失效，日志会提示写入 connect.json 持久化）。
    token: RwLock<String>,
    /// 游标与登录二维码的落盘目录（`{data_dir}/connect_wechat/`）。
    state_dir: Option<PathBuf>,
    /// 语音能力配置（`voice` 段；无 `siliconflow_api_key` 时为 `None`，
    /// 全部语音行为关闭，与未配置时完全一致）。
    voice: Option<VoiceConfig>,
    /// 入站观测到的语音 `encode_type`（0 = 尚未观测）。微信自己的语音条
    /// （SILK 载荷）携带的取值即本线格式下 SILK 的真实值，出站语音
    /// 自动采用（见 [`VOICE_ENCODE_TYPE_SILK`] 的矛盾说明）。
    observed_voice_encode_type: AtomicI64,
    /// 入站语音取证探针只跑一次（dump JSON + 解密字节落盘）。
    voice_probe_done: AtomicBool,
    /// 探针捕获的入站语音 media 引用与字段（诊断回显模式用：把微信
    /// 自己的语音条原样发回去，判定通道是否支持出站语音条渲染）。
    captured_inbound_voice: RwLock<Option<CapturedVoiceMedia>>,
    /// `get_updates_buf` 游标（内存权威副本；每次成功拉取后落盘）。
    cursor: AsyncMutex<Option<String>>,
    rate_limiter: RateLimiter,
}

impl WechatPlatform {
    /// 生产构造：官方网关 + 默认限流。
    pub fn new(
        token: String,
        base_url: String,
        state_dir: Option<PathBuf>,
        voice: Option<VoiceConfig>,
    ) -> Self {
        Self::with_options(token, base_url, DEFAULT_RATE_LIMIT_INTERVAL, state_dir, voice)
    }

    /// 测试/高级构造：可注入本地 HTTP 桩地址与极小的限流间隔。
    pub fn with_options(
        token: String,
        base_url: String,
        rate_limit_interval: Duration,
        state_dir: Option<PathBuf>,
        voice: Option<VoiceConfig>,
    ) -> Self {
        Self {
            base_url: RwLock::new(base_url),
            cdn_base_url: DEFAULT_CDN_BASE_URL.to_string(),
            token: RwLock::new(token),
            state_dir,
            voice,
            observed_voice_encode_type: AtomicI64::new(0),
            voice_probe_done: AtomicBool::new(false),
            captured_inbound_voice: RwLock::new(None),
            cursor: AsyncMutex::new(None),
            rate_limiter: RateLimiter::new(rate_limit_interval),
        }
    }

    /// 测试专用：注入本地 CDN 桩地址。
    #[cfg(test)]
    fn with_cdn_base(mut self, cdn_base_url: String) -> Self {
        self.cdn_base_url = cdn_base_url;
        self
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

        let mut events = Vec::with_capacity(parsed.msgs.len());
        for (index, msg) in parsed.msgs.iter().enumerate() {
            if msg.message_type != MSG_TYPE_INBOUND {
                continue;
            }
            // 内容行：文本 + 语音转写在映射时同步生成；图片/文件/视频需要
            // 异步下载，先落盘再把路径追加为文本行（Agent 可用工具打开）。
            let (mut lines, media_items) = extract_content_lines(msg);
            // 出站 mirror 决策需要知道本条入站是否含语音条。
            let had_voice = msg
                .item_list
                .iter()
                .any(|item| item.kind == Some(ITEM_TYPE_VOICE));
            // 观测微信自身语音条的 encode_type：微信语音条是 SILK 载荷，
            // 它带的值就是本线格式下 SILK 的真实取值（出站自动校准用）。
            if had_voice {
                for item in &msg.item_list {
                    if let Some(observed) = item
                        .voice_item
                        .as_ref()
                        .and_then(|voice| voice.encode_type)
                    {
                        let previous = self.observed_voice_encode_type.swap(observed, Ordering::Relaxed);
                        if previous != observed {
                            tracing::info!(
                                "connect: wechat observed inbound voice encode_type={observed} \
                                 (will use for outbound voice)"
                            );
                        }
                    }
                    if let Some(voice) = item.voice_item.as_ref() {
                        self.debug_probe_inbound_voice(voice).await;
                    }
                }
            }
            // 媒体即使下载失败也保留占位行——纯媒体消息不能因为 CDN
            // 抖动就整条消失。
            for media in &media_items {
                match self.fetch_and_save_media(media, index).await {
                    Ok(path) => {
                        let label = media.label();
                        let file_name = media.file_name().map(str::to_string);
                        match &file_name {
                            Some(name) => {
                                lines.push(format!("{label} {}（{name}）", path.display()))
                            }
                            None => lines.push(format!("{label} {}", path.display())),
                        }
                        // 音频文件转写（file_asr 开启时）：已解密的原始文件
                        // 直接上传识别，成功追加 [音频转写] 行；音乐/无语音/
                        // 超限/接口失败一律静默跳过，文件标记不受影响。
                        if let Some(name) = &file_name {
                            self.maybe_transcribe_audio(&path, name, &mut lines)
                                .await;
                        }
                    }
                    Err(error) => {
                        tracing::warn!("connect: wechat inbound media failed: {error}");
                        lines
                            .push(format!("{}（下载或解密失败，未能保存）", media.label()));
                    }
                }
            }
            if let Some(message) = Self::build_inbound_message(msg, index, lines, had_voice) {
                events.push(Inbound::Message(message));
            }
        }
        Ok(events)
    }

    /// 把一条 iLink 消息映射为 bridge 的 [`InboundMessage`]，消息文本取
    /// `lines`（文本行 + 语音转写行 + 图片路径行，由调用方组装）。
    ///
    /// 返回 `None` 的消息（出站回显、无内容、无 `from_user_id`、无
    /// `context_token`）只是不转发——游标已在 `poll_once` 里推进，网关不会
    /// 重发它们（这点与 telegram 的 offset 语义一致）。
    fn build_inbound_message(
        msg: &IlinkMessage,
        batch_index: usize,
        lines: Vec<String>,
        had_voice: bool,
    ) -> Option<InboundMessage> {
        let from = msg
            .from_user_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())?;
        let text = lines.join("\n");
        if text.trim().is_empty() {
            return None;
        }
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
                // 出站 mirror 语音决策用：本条入站消息是否含语音条。
                "had_voice": had_voice,
            })),
        })
    }

    /// 入站音频文件转写（`voice.file_asr` 开启时）：把已解密落盘的音频
    /// 上传硅基流动识别，成功则向 `lines` 追加 `[音频转写] <文本>` 行。
    /// 音乐/无语音内容/超限/接口失败不阻塞主流程（文件路径行始终保留），
    /// 但**失败记 WARN**——静默跳过只对用户体验，排查必须有痕。微信
    /// **语音条**不走这里（永远用微信自带转写，见设计文档 §4）。
    async fn maybe_transcribe_audio(
        &self,
        path: &std::path::Path,
        file_name: &str,
        lines: &mut Vec<String>,
    ) {
        let Some(voice) = self.voice.as_ref() else {
            return;
        };
        if !voice.file_asr || !wechat_voice::is_audio_file_name(file_name) {
            return;
        }
        let bytes = match tokio::fs::read(path).await {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!("connect: wechat audio asr read failed: {error}");
                return;
            }
        };
        if bytes.len() > wechat_voice::MAX_ASR_FILE_BYTES {
            tracing::warn!(
                "connect: wechat audio asr skipped ({} bytes over cap)",
                bytes.len()
            );
            return;
        }
        match wechat_voice::transcribe_file(http_client(), voice, &bytes, file_name).await {
            Ok(text) if !text.is_empty() => {
                tracing::info!(
                    "connect: wechat audio asr ok file={file_name} chars={}",
                    text.chars().count()
                );
                lines.push(format!("[音频转写] {text}"));
            }
            Ok(_) => {
                tracing::warn!("connect: wechat audio asr empty result for {file_name}");
            }
            Err(error) => {
                tracing::warn!("connect: wechat audio asr failed for {file_name}: {error}");
            }
        }
    }

    /// 下载并解密一个入站媒体（图片/文件/视频），保存到 `state_dir/media/`
    /// 下，返回落盘路径。
    ///
    /// 流程对齐 cc-connect 的 cdn.go：
    /// 入站语音取证探针（每进程一次，出站语音真机调试用）：完整记录
    /// 微信自己语音条的字段形状，并下载解密载荷字节落盘，供与出站
    /// SILK 字节做逐层对比（头部变体 / 帧结构 / 采样率）。失败只记
    /// WARN，绝不影响正常消息流。
    async fn debug_probe_inbound_voice(&self, voice: &IlinkVoiceItem) {
        if self.voice_probe_done.swap(true, Ordering::Relaxed) {
            return;
        }
        tracing::info!(
            "connect: wechat inbound voice probe: text={:?} encode_type={:?} \
             playtime={:?} sample_rate={:?} bits_per_sample={:?}",
            voice.text,
            voice.encode_type,
            voice.playtime,
            voice.sample_rate,
            voice.bits_per_sample,
        );
        // 捕获完整字段供诊断回显（BAMBOO_WECHAT_VOICE_ECHO=1 时原样发回）。
        if let Some(descriptor) = voice.media.as_ref() {
            // 入站 aes_key 形态：b64 解码 16 字节=raw 形态，32 字节=hex-ASCII
            // 形态（我们的出站上传用的是后者——文件通道真机可用）。
            let key_form = descriptor.aes_key.as_deref().map(|key| {
                match base64::engine::general_purpose::STANDARD.decode(key.trim().as_bytes()) {
                    Ok(decoded) => match decoded.len() {
                        16 => "b64(raw16)".to_string(),
                        32 => "b64(hex32-ascii)".to_string(),
                        other => format!("b64({other}B?)"),
                    },
                    Err(_) => "not-base64".to_string(),
                }
            });
            tracing::info!(
                "connect: wechat inbound voice probe: aes_key form={key_form:?} \
                 (outbound upload uses b64(hex32-ascii))"
            );
            let mut media = serde_json::Map::new();
            media.insert(
                "encrypt_query_param".to_string(),
                serde_json::Value::String(descriptor.encrypt_query_param.clone().unwrap_or_default()),
            );
            if let Some(key) = descriptor.aes_key.clone() {
                media.insert("aes_key".to_string(), serde_json::Value::String(key));
            }
            if let Some(encrypt_type) = descriptor.encrypt_type {
                media.insert("encrypt_type".to_string(), serde_json::Value::from(encrypt_type));
            }
            *self.captured_inbound_voice.write().expect("voice capture lock") = Some(CapturedVoiceMedia {
                media: serde_json::Value::Object(media),
                encode_type: voice.encode_type,
                playtime: voice.playtime,
                sample_rate: voice.sample_rate,
                bits_per_sample: voice.bits_per_sample,
            });
        }
        let Some(descriptor) = voice.media.as_ref() else {
            tracing::warn!("connect: wechat inbound voice probe: no media descriptor");
            return;
        };
        let Some(enc_param) = descriptor
            .encrypt_query_param
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            tracing::warn!("connect: wechat inbound voice probe: no encrypt_query_param");
            return;
        };
        let Some(dir) = self.state_dir.as_deref() else {
            tracing::warn!("connect: wechat inbound voice probe: no state dir");
            return;
        };
        // 回显模式下跳过下载：CDN 下载凭据可能是一次性的，探针下载会把
        // 凭据消费掉，导致回显的 media 在客户端拉取失败（首轮回显实验
        // 的无效性正来自这里）。取证字节等回显实验结束后再补。
        let echo_mode = std::env::var("BAMBOO_WECHAT_VOICE_ECHO")
            .map(|value| value.trim() == "1")
            .unwrap_or(false);
        if echo_mode {
            tracing::info!(
                "connect: wechat inbound voice probe: ECHO mode active, skipping media download \
                 to keep the credential fresh for the echo"
            );
            return;
        }
        let url = format!(
            "{}/download?encrypted_query_param={}",
            self.cdn_base_url.trim_end_matches('/'),
            percent_encode_query(enc_param)
        );
        let ciphertext = match http_client()
            .get(&url)
            .timeout(MEDIA_DOWNLOAD_TIMEOUT)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => response.bytes().await,
            Ok(response) => {
                tracing::warn!(
                    "connect: wechat inbound voice probe: CDN returned HTTP {}",
                    response.status()
                );
                return;
            }
            Err(error) => {
                tracing::warn!("connect: wechat inbound voice probe: CDN failed: {}", error.without_url());
                return;
            }
        };
        let ciphertext = match ciphertext {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!("connect: wechat inbound voice probe: CDN read failed: {error}");
                return;
            }
        };
        let key = normalize_media_key(None, descriptor.aes_key.as_deref());
        let plain = match key {
            Some(key) => decrypt_aes_128_ecb(&key, &ciphertext),
            None => {
                tracing::warn!("connect: wechat inbound voice probe: no parsable aes_key, saving raw ciphertext");
                Some(ciphertext.to_vec())
            }
        };
        let Some(plain) = plain else {
            tracing::warn!("connect: wechat inbound voice probe: decrypt failed");
            return;
        };
        let head: String = plain
            .iter()
            .take(32)
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        tracing::info!(
            "connect: wechat inbound voice probe: media encrypt_type={:?} \
             has_key={} bytes={} head_hex={head}",
            descriptor.encrypt_type,
            key.is_some(),
            plain.len(),
        );
        let path = dir.join("inbound_voice_probe.bin");
        if let Err(error) = std::fs::write(&path, &plain) {
            tracing::warn!("connect: wechat inbound voice probe: write failed: {error}");
            return;
        }
        tracing::info!("connect: wechat inbound voice probe saved: {}", path.display());
    }


    /// （图片密钥优先取 `aeskey` hex 字段，否则 base64 的 `media.aes_key`）
    /// → 落盘。无密钥时走明文直下兜底（cc-connect 对图片同款）。
    async fn fetch_and_save_media(
        &self,
        media: &InboundMedia,
        batch_index: usize,
    ) -> PlatformResult<PathBuf> {
        let dir = self
            .state_dir
            .as_deref()
            .ok_or_else(|| PlatformError::other("wechat media requires a state dir"))?
            .join("media");
        let descriptor = media.media().ok_or_else(|| {
            PlatformError::other("weixin media item is missing its media descriptor")
        })?;
        let enc_param = descriptor
            .encrypt_query_param
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| PlatformError::other("weixin media is missing encrypt_query_param"))?;

        let url = format!(
            "{}/download?encrypted_query_param={}",
            self.cdn_base_url.trim_end_matches('/'),
            percent_encode_query(enc_param)
        );
        let response = http_client()
            .get(&url)
            .timeout(MEDIA_DOWNLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|error| {
                // CDN 参数不是秘密，但保持与网关错误同款的脱敏习惯。
                PlatformError::other(format!(
                    "media CDN request failed: {}",
                    error.without_url()
                ))
            })?;
        if !response.status().is_success() {
            return Err(PlatformError::other(format!(
                "media CDN returned HTTP {}",
                response.status()
            )));
        }
        let ciphertext = response
            .bytes()
            .await
            .map_err(|error| PlatformError::other(format!("media CDN read failed: {error}")))?;
        if ciphertext.len() > MAX_MEDIA_BYTES {
            return Err(PlatformError::other("media exceeds the 100 MB cap"));
        }

        let plain = match normalize_media_key(media.media_key(), descriptor.aes_key.as_deref()) {
            Some(key) => decrypt_aes_128_ecb(&key, &ciphertext).ok_or_else(|| {
                PlatformError::other("media AES-128-ECB decrypt failed (bad key or padding)")
            })?,
            // 无密钥：cc-connect 对图片走明文直下兜底。
            None => ciphertext.to_vec(),
        };
        if plain.is_empty() {
            return Err(PlatformError::other("decrypted media is empty"));
        }

        std::fs::create_dir_all(&dir).map_err(|error| {
            PlatformError::other(format!("media dir create failed: {error}"))
        })?;
        // 图片按魔数嗅探扩展名，文件/视频按原始名/默认扩展名。
        let ext = match media {
            InboundMedia::Image(_) => sniff_image_ext(&plain).to_string(),
            _ => media.default_ext(),
        };
        let path = dir.join(format!(
            "wechat_media_{}_{batch_index}.{ext}",
            chrono::Utc::now().timestamp_millis(),
        ));
        std::fs::write(&path, &plain)
            .map_err(|error| PlatformError::other(format!("media write failed: {error}")))?;
        Ok(path)
    }

    /// 发送一条文本消息（`reply` 对每个分块调用一次）。线格式对齐 cc-connect
    /// 的 `sendMessageReq`：请求根级必须带 `base_info`，msg 必须带 `from_user_id`
    /// （bot 发送时为空串）与 `client_id`（客户端生成的消息 id）——缺这些字段
    /// 时网关可能返回 ret=0 却不投递消息（静默失败）。
    async fn send_message(&self, to_user_id: &str, context_token: &str, text: &str) -> PlatformResult<()> {
        let item = serde_json::json!({
            "type": ITEM_TYPE_TEXT,
            "text_item": { "text": text },
        });
        self.send_message_item(to_user_id, context_token, item).await
    }

    /// 发送一条携带任意 `item_list` 条目的消息（文本/图片/文件/视频共用）。
    async fn send_message_item(
        &self,
        to_user_id: &str,
        context_token: &str,
        item: serde_json::Value,
    ) -> PlatformResult<()> {
        let url = format!("{}/ilink/bot/sendmessage", self.base_url());
        let body = serde_json::json!({
            "msg": {
                "from_user_id": "",
                "to_user_id": to_user_id,
                "client_id": uuid::Uuid::new_v4().to_string(),
                "message_type": MSG_TYPE_OUTBOUND,
                "message_state": MSG_STATE_FINISH,
                "context_token": context_token,
                "item_list": [ item ],
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

        // 观测点：微信网关存在 ret=0 却不投递的静默丢弃，出站问题排查必须
        // 能看到每次发送的实际返回。
        tracing::info!(
            "connect: wechat sendmessage to={to_user_id} ret={} errcode={} errmsg={:?}",
            parsed.ret,
            parsed.errcode,
            parsed.errmsg,
        );

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

    /// 把一个本地文件投递给微信用户（出站附件，对齐 cc-connect 的
    /// media_outbound.go 流程）：
    ///
    /// 1. `POST /ilink/bot/getuploadurl`（filekey 随机、aeskey 为 hex、
    ///    filesize 为填充后密文长度、rawfilemd5 为明文 MD5）；
    /// 2. 密文 `POST` 到 CDN（优先响应里的 `upload_full_url`，否则
    ///    `{cdn}/upload?encrypted_query_param=...&filekey=...`），下载凭据
    ///    从响应头 `x-encrypted-param` 读取；
    /// 3. `sendmessage` 按媒体类型携带 image_item / video_item / file_item
    ///    （media.aes_key 用 `base64(hex)` 形态）。
    async fn deliver_file(
        &self,
        to_user_id: &str,
        context_token: &str,
        path: &std::path::Path,
    ) -> PlatformResult<()> {
        let plain = tokio::fs::read(path)
            .await
            .map_err(|error| PlatformError::other(format!("read {}: {error}", path.display())))?;
        if plain.len() > MAX_MEDIA_BYTES {
            return Err(PlatformError::other(
                "file exceeds the 100 MB delivery cap",
            ));
        }

        // 媒体分类：图片按魔数，视频按 MP4 ftyp 盒，其余一律按文件。
        let (media_type, item_type) = if is_raster_image(&plain) {
            (UPLOAD_MEDIA_IMAGE, ITEM_TYPE_IMAGE)
        } else if plain.len() >= 12 && &plain[4..8] == b"ftyp" {
            (UPLOAD_MEDIA_VIDEO, ITEM_TYPE_VIDEO)
        } else {
            (UPLOAD_MEDIA_FILE, ITEM_TYPE_FILE)
        };

        let (media, padded_size) = self.upload_media(to_user_id, &plain, media_type).await?;

        // 3. 发送携带媒体条目的消息。
        let item = match item_type {
            ITEM_TYPE_IMAGE => serde_json::json!({
                "type": ITEM_TYPE_IMAGE,
                "image_item": { "media": media, "mid_size": padded_size }
            }),
            ITEM_TYPE_VIDEO => serde_json::json!({
                "type": ITEM_TYPE_VIDEO,
                "video_item": { "media": media, "video_size": padded_size }
            }),
            _ => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "file.bin".to_string());
                serde_json::json!({
                    "type": ITEM_TYPE_FILE,
                    "file_item": { "media": media, "file_name": name, "len": plain.len().to_string() }
                })
            }
        };
        self.send_message_item(to_user_id, context_token, item).await
    }

    /// 出站媒体上传公共链路（deliver_file / deliver_voice 共用）：
    /// 随机 AES key 加密 → `getuploadurl`（语音同走 FILE 通道）→ 密文
    /// POST 到 CDN → 返回（`media` 引用 JSON，填充后密文长度）。
    async fn upload_media(
        &self,
        to_user_id: &str,
        plain: &[u8],
        media_type: i64,
    ) -> PlatformResult<(serde_json::Value, usize)> {
        let key: [u8; 16] = rand::random();
        let ciphertext = encrypt_aes_128_ecb(&key, plain);
        let padded_size = ciphertext.len();
        let filekey = random_hex_16();
        let hex_key: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
        let md5_hex = {
            use md5::Digest;
            let digest = md5::Md5::digest(plain);
            digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>()
        };

        // 1. getuploadurl
        let url = format!("{}/ilink/bot/getuploadurl", self.base_url());
        let body = serde_json::json!({
            "filekey": filekey,
            "media_type": media_type,
            "to_user_id": to_user_id,
            "rawsize": plain.len(),
            "rawfilemd5": md5_hex,
            "filesize": ciphertext.len(),
            "no_need_thumb": true,
            "aeskey": hex_key,
            "base_info": { "channel_version": CHANNEL_VERSION },
        });
        let response = self
            .ilink_request(reqwest::Method::POST, &url)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                PlatformError::other(format!(
                    "getuploadurl request failed: {}",
                    self.sanitize_error(error)
                ))
            })?;
        let parsed: GetUploadUrlResponse = response.json().await.map_err(|error| {
            PlatformError::other(format!(
                "getuploadurl response parse failed: {}",
                self.sanitize_error(error)
            ))
        })?;
        if parsed.ret != 0 || parsed.errcode != 0 {
            return Err(ret_error_full(
                "getuploadurl",
                parsed.ret,
                parsed.errcode,
                parsed.errmsg,
            ));
        }

        // 2. 上传密文；下载凭据在响应头 x-encrypted-param。
        let upload_url = match parsed
            .upload_full_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(full) => full.to_string(),
            None => {
                let param = parsed
                    .upload_param
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        PlatformError::other(
                            "getuploadurl returned neither upload_full_url nor upload_param",
                        )
                    })?;
                format!(
                    "{}/upload?encrypted_query_param={}&filekey={}",
                    self.cdn_base_url.trim_end_matches('/'),
                    percent_encode_query(param),
                    percent_encode_query(&filekey)
                )
            }
        };
        let response = http_client()
            .post(&upload_url)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .timeout(MEDIA_DOWNLOAD_TIMEOUT)
            .body(ciphertext)
            .send()
            .await
            .map_err(|error| {
                PlatformError::other(format!("media upload failed: {}", error.without_url()))
            })?;
        if !response.status().is_success() {
            return Err(PlatformError::other(format!(
                "media upload returned HTTP {}",
                response.status()
            )));
        }
        let download_param = response
            .headers()
            .get("x-encrypted-param")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                PlatformError::other("media upload response missing x-encrypted-param header")
            })?
            .to_string();

        let media = serde_json::json!({
            "encrypt_type": 1,
            "encrypt_query_param": download_param,
            "aes_key": format_media_key_for_api(&key),
        });
        Ok((media, padded_size))
    }

    /// 把一段文本合成为微信语音条发送（TTS → SILK 编码 → CDN 上传 →
    /// sendmessage voice_item）。超长文本先按句读分段，每段一条语音，
    /// 段数上限 [`wechat_voice::MAX_VOICE_SEGMENTS`]；单段超过 60 秒按
    /// 字节截断。任何失败向上返回（调用方负责文字回退）。
    async fn deliver_voice(
        &self,
        to_user_id: &str,
        context_token: &str,
        text: &str,
    ) -> PlatformResult<()> {
        let voice = self
            .voice
            .as_ref()
            .ok_or_else(|| PlatformError::other("voice is not configured"))?;
        // 诊断回显模式：跳过 TTS/编码/上传，把探针捕获的入站语音 media
        // 引用与字段原样发回。字节级一致仍不渲染 ⇒ 通道不支持出站语音条；
        // 渲染了 ⇒ 问题在我们上传的 media（密钥形态/桶），对比日志修。
        if std::env::var("BAMBOO_WECHAT_VOICE_ECHO")
            .map(|value| value.trim() == "1")
            .unwrap_or(false)
        {
            let captured = self
                .captured_inbound_voice
                .read()
                .expect("voice capture lock")
                .clone();
            let Some(captured) = captured else {
                return Err(PlatformError::other(
                    "回显模式：尚未捕获入站语音，请先发一条微信语音条再触发语音回复",
                ));
            };
            let mut voice_item = serde_json::Map::new();
            voice_item.insert("media".to_string(), captured.media);
            voice_item.insert(
                "encode_type".to_string(),
                serde_json::Value::from(
                    captured
                        .encode_type
                        .unwrap_or_else(|| self.resolve_outbound_voice_encode_type()),
                ),
            );
            if let Some(sample_rate) = captured.sample_rate {
                voice_item.insert("sample_rate".to_string(), serde_json::Value::from(sample_rate));
            }
            if let Some(bits) = captured.bits_per_sample {
                voice_item.insert("bits_per_sample".to_string(), serde_json::Value::from(bits));
            }
            if let Some(playtime) = captured.playtime {
                voice_item.insert("playtime".to_string(), serde_json::Value::from(playtime));
            }
            let item = serde_json::json!({
                "type": ITEM_TYPE_VOICE,
                "voice_item": serde_json::Value::Object(voice_item),
            });
            tracing::warn!(
                "connect: wechat voice ECHO mode: sending captured inbound voice verbatim"
            );
            self.rate_limiter.wait(to_user_id).await;
            return self.send_message_item(to_user_id, context_token, item).await;
        }
        let normalized = wechat_voice::normalize_for_tts(text);
        // 默认 file 投递：TTS 直接合成 mp3 → FILE 通道上传 → file_item
        // （iLink 不渲染 bot 的 voice_item，见 VoiceDelivery 注释）。
        if voice.delivery == wechat_voice::VoiceDelivery::File {
            if normalized.is_empty() {
                return Err(PlatformError::other("语音文本规范化后为空"));
            }
            let mp3 = wechat_voice::synthesize_mp3(http_client(), voice, &normalized)
                .await
                .map_err(PlatformError::other)?;
            let (media, _padded) = self
                .upload_media(to_user_id, &mp3, UPLOAD_MEDIA_FILE)
                .await?;
            let file_name = format!(
                "语音回复-{}.mp3",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or_default()
            );
            let item = serde_json::json!({
                "type": ITEM_TYPE_FILE,
                "file_item": {
                    "media": media,
                    "file_name": file_name,
                    "len": mp3.len().to_string(),
                }
            });
            tracing::info!(
                "connect: wechat voice(file) to={to_user_id} chars={} mp3_bytes={}",
                normalized.chars().count(),
                mp3.len(),
            );
            self.rate_limiter.wait(to_user_id).await;
            return self.send_message_item(to_user_id, context_token, item).await;
        }
        // bubble 投递（实验）：SILK 语音条路径。
        let segments = wechat_voice::segment_for_tts(
            &normalized,
            wechat_voice::MAX_VOICE_SEGMENT_CHARS,
        );
        if segments.is_empty() {
            return Err(PlatformError::other("语音文本规范化后为空"));
        }
        if segments.len() > wechat_voice::MAX_VOICE_SEGMENTS {
            tracing::warn!(
                "connect: wechat voice text split into {} segments, truncating to {}",
                segments.len(),
                wechat_voice::MAX_VOICE_SEGMENTS
            );
        }
        for segment in segments.into_iter().take(wechat_voice::MAX_VOICE_SEGMENTS) {
            let pcm = wechat_voice::synthesize_pcm(http_client(), voice, &segment)
                .await
                .map_err(PlatformError::other)?;
            // 60 秒上限：按字节截断 PCM（对齐帧边界由编码器的补零逻辑兜底）。
            let max_bytes =
                (voice.sample_rate as usize * 2) * (wechat_voice::MAX_VOICE_DURATION_MS as usize / 1000);
            let pcm = if pcm.len() > max_bytes { &pcm[..max_bytes] } else { &pcm[..] };
            let playtime_ms = wechat_voice::pcm_duration_ms(pcm.len(), voice.sample_rate);
            let mut silk = wechat_voice::encode_silk_tencent(pcm, voice.sample_rate, voice.bitrate)
                .map_err(PlatformError::other)?;
            // 载荷变体排查开关：微信语音条的腾讯变体带 \x02 前缀；若真机
            // 仍不显示，用 BAMBOO_WECHAT_VOICE_NO_SILK_PREFIX=1 试去掉前缀
            // 的标准 SILK v3 形态（与入站取证字节对比后决定默认值）。
            if std::env::var("BAMBOO_WECHAT_VOICE_NO_SILK_PREFIX")
                .map(|value| value.trim() == "1")
                .unwrap_or(false)
                && silk.first() == Some(&0x02)
            {
                silk.remove(0);
                tracing::warn!("connect: wechat voice silk \\x02 prefix stripped by env override");
            }
            // 上传走 FILE 通道（VOICE=4 真机静默丢弃，见 UPLOAD_MEDIA_VOICE
            // 注释；cc-connect 真机可用实现同样用 FILE 上传语音）。
            let (media, _padded) = self
                .upload_media(to_user_id, &silk, UPLOAD_MEDIA_FILE)
                .await?;
            // voice_item 字段形状对齐 2026-10-05 入站取证（微信自己的语音
            // 条）：media + encode_type + sample_rate + bits_per_sample +
            // playtime(毫秒)。此前只发 media+encode_type，真机不渲染。
            let encode_type = self.resolve_outbound_voice_encode_type();
            let item = serde_json::json!({
                "type": ITEM_TYPE_VOICE,
                "voice_item": {
                    "media": media,
                    "encode_type": encode_type,
                    "sample_rate": voice.sample_rate,
                    "bits_per_sample": 16,
                    "playtime": playtime_ms,
                }
            });
            tracing::info!(
                "connect: wechat voice to={to_user_id} chars={} silk_bytes={} playtime_ms={playtime_ms} encode_type={encode_type}",
                segment.chars().count(),
                silk.len(),
            );
            self.rate_limiter.wait(to_user_id).await;
            self.send_message_item(to_user_id, context_token, item).await?;
        }
        Ok(())
    }

    /// 出站语音 `encode_type` 三级解析：环境变量强制覆盖（排查用）>
    /// 入站观测值（微信自己的语音条携带值，自动校准）> 官方包注释兜底
    /// （6=silk）。资料矛盾与真机丢弃记录见 [`VOICE_ENCODE_TYPE_SILK`]。
    fn resolve_outbound_voice_encode_type(&self) -> i64 {
        if let Ok(raw) = std::env::var("BAMBOO_WECHAT_VOICE_ENCODE_TYPE") {
            if let Ok(value) = raw.trim().parse::<i64>() {
                tracing::warn!(
                    "connect: wechat voice encode_type={value} forced by \
                     BAMBOO_WECHAT_VOICE_ENCODE_TYPE"
                );
                return value;
            }
        }
        let observed = self
            .observed_voice_encode_type
            .load(Ordering::Relaxed);
        if observed != 0 {
            observed
        } else {
            VOICE_ENCODE_TYPE_SILK
        }
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

/// 拆解一条消息的内容：文本条目与语音转写生成文本行（语音无转写时也保留
/// 占位行），图片/文件/视频条目收集起来交给调用方异步下载（`poll_once`）。
enum InboundMedia {
    Image(IlinkImageItem),
    File(IlinkFileItem),
    Video(IlinkVideoItem),
}

impl InboundMedia {
    /// CDN 媒体描述符与对应的 AES 密钥字段。
    fn media(&self) -> Option<&IlinkCdnMedia> {
        match self {
            InboundMedia::Image(item) => item.media.as_ref(),
            InboundMedia::File(item) => item.media.as_ref(),
            InboundMedia::Video(item) => item.media.as_ref(),
        }
    }

    fn media_key(&self) -> Option<&str> {
        match self {
            InboundMedia::Image(item) => item.aes_key_hex.as_deref(),
            _ => None,
        }
    }

    /// 展示前缀与落盘默认扩展名。
    fn label(&self) -> &'static str {
        match self {
            InboundMedia::Image(_) => "[图片]",
            InboundMedia::File(_) => "[文件]",
            InboundMedia::Video(_) => "[视频]",
        }
    }

    fn default_ext(&self) -> String {
        match self {
            InboundMedia::Image(_) => "jpg".to_string(),
            InboundMedia::Video(_) => "mp4".to_string(),
            InboundMedia::File(item) => item
                .file_name
                .as_deref()
                .and_then(|name| name.rsplit_once('.'))
                .map(|(_, ext)| ext)
                .filter(|ext| !ext.is_empty() && ext.chars().count() <= 10)
                .unwrap_or("bin")
                .to_string(),
        }
    }

    fn file_name(&self) -> Option<&str> {
        match self {
            InboundMedia::File(item) => item.file_name.as_deref(),
            _ => None,
        }
    }
}

fn extract_content_lines(msg: &IlinkMessage) -> (Vec<String>, Vec<InboundMedia>) {
    let mut lines = Vec::new();
    let mut media = Vec::new();

    for item in &msg.item_list {
        match item.kind {
            Some(ITEM_TYPE_TEXT) => {
                if let Some(text) = item
                    .text_item
                    .as_ref()
                    .and_then(|text_item| text_item.text.as_deref())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    lines.push(text.to_string());
                }
            }
            Some(ITEM_TYPE_VOICE) => {
                // 微信 ASR 转写在 voice_item.text；有转写就不下载 SILK 音频。
                match item
                    .voice_item
                    .as_ref()
                    .and_then(|voice| voice.text.as_deref())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    Some(text) => lines.push(format!("[语音] {text}")),
                    None => lines.push("[语音]（无转写文本）".to_string()),
                }
            }
            Some(ITEM_TYPE_IMAGE) => {
                if let Some(image) = item.image_item.as_ref() {
                    media.push(InboundMedia::Image(IlinkImageItem {
                        media: image.media.clone(),
                        aes_key_hex: image.aes_key_hex.clone(),
                    }));
                }
            }
            Some(ITEM_TYPE_FILE) => {
                if let Some(file) = item.file_item.as_ref() {
                    media.push(InboundMedia::File(IlinkFileItem {
                        media: file.media.clone(),
                        file_name: file.file_name.clone(),
                    }));
                }
            }
            Some(ITEM_TYPE_VIDEO) => {
                if let Some(video) = item.video_item.as_ref() {
                    media.push(InboundMedia::Video(IlinkVideoItem {
                        media: video.media.clone(),
                    }));
                }
            }
            _ => {}
        }
    }

    (lines, media)
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
            // 出站附件投递：解析回复中的 [SEND_FILE: 路径] 标记并经
            // iLink CDN 上传发送（bridge 据此在会话首条消息注入约定说明）。
            attachments: true,
            // 微信 IM 场景只看结果：不推送 ⚙ 工具命令行进度。
            tool_progress: false,
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
        //
        // 出站附件与语音：先剥离 [SEND_FILE: 路径] 与 [SEND_VOICE: 文本]
        // 标记行；再按 reply_mode 决定普通回复是否转语音（决策管线见
        // 设计文档 §3a：off=总开关默认关 / mirror=语音回语音 / always）。
        // 语音失败回退文字，绝不静默卡住。
        let (visible_after_files, files) = extract_send_file_markers(&msg.text);
        let (mut visible_text, voice_markers) = extract_send_voice_markers(&visible_after_files);
        // 剥离标记后可见文本为空但确有附件时，补一条兜底文案——否则用户
        // 只看到 ⚙ 工具行，永远等不到正文（"只见命令不见回复"）。
        if visible_text.trim().is_empty() && !files.is_empty() && voice_markers.is_empty() {
            let names: Vec<String> = files
                .iter()
                .map(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.display().to_string())
                })
                .collect();
            visible_text = format!("📄 已为你发送 {} 个文件：{}", files.len(), names.join("、"));
        }

        // 语音未配置时标记文本并入正文（模型误用 [SEND_VOICE] 也不丢内容，
        // 而不是报"语音合成失败"）。
        if self.voice.is_none() && !voice_markers.is_empty() {
            let marker_text = voice_markers.join("\n");
            visible_text = if visible_text.is_empty() {
                marker_text
            } else {
                format!("{visible_text}\n{marker_text}")
            };
        }

        // 自动（非显式标记）语音决策：网关确定性规则，不依赖模型自觉。
        // mirror 需要入站消息含语音（reply_ctx.had_voice，poll_once 写入）。
        let had_voice = ctx
            .0
            .get("had_voice")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let auto_voice_text = match self.voice.as_ref().map(|voice| voice.reply_mode) {
            Some(wechat_voice::VoiceReplyMode::Mirror)
                if had_voice && wechat_voice::is_voice_suitable(&visible_text) =>
            {
                Some(visible_text.clone())
            }
            Some(wechat_voice::VoiceReplyMode::Always)
                if wechat_voice::is_voice_suitable(&visible_text) =>
            {
                Some(visible_text.clone())
            }
            _ => None,
        };
        // 自动语音命中时整条可见文本转为语音条，不再重复发文字（内容门槛
        // 已保证适合念）；显式标记模式可见文本照常发送。
        if auto_voice_text.is_some() {
            visible_text = String::new();
        }

        tracing::info!(
            "connect: wechat reply to={to_user_id} visible_chars={} attachments={} voice_markers={} auto_voice={}",
            visible_text.chars().count(),
            files.len(),
            voice_markers.len(),
            auto_voice_text.is_some()
        );
        for chunk in chunk_message(&visible_text, WECHAT_MESSAGE_CHARS) {
            self.rate_limiter.wait(&to_user_id).await;
            self.send_message(&to_user_id, &context_token, &chunk).await?;
        }
        for path in files {
            self.rate_limiter.wait(&to_user_id).await;
            if let Err(error) = self
                .deliver_file(&to_user_id, &context_token, &path)
                .await
            {
                tracing::warn!(
                    "connect: wechat file delivery failed for {}: {error}",
                    path.display()
                );
                let notice =
                    format!("[文件发送失败：{}] {error}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                self.rate_limiter.wait(&to_user_id).await;
                let _ = self
                    .send_message(&to_user_id, &context_token, &notice)
                    .await;
            }
        }

        // 语音条：显式标记逐条念出；自动模式整条转换。失败回退文字
        // （自动模式原文还没发过，回退时把原文带上）。
        let auto_voice_active = auto_voice_text.is_some();
        let mut voice_texts = voice_markers;
        if let Some(text) = auto_voice_text {
            voice_texts.push(text);
        }
        for text in voice_texts {
            if let Err(error) = self
                .deliver_voice(&to_user_id, &context_token, &text)
                .await
            {
                tracing::warn!("connect: wechat voice delivery failed: {error}");
                let fallback = if auto_voice_active {
                    format!("（语音合成失败，已改用文字：{error}）\n{text}")
                } else {
                    format!("（语音合成失败：{error}）\n{text}")
                };
                for chunk in chunk_message(&fallback, WECHAT_MESSAGE_CHARS) {
                    self.rate_limiter.wait(&to_user_id).await;
                    let _ = self
                        .send_message(&to_user_id, &context_token, &chunk)
                        .await;
                }
            }
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
            None,
        );
        first.poll_once().await.unwrap();

        let restarted = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            server.uri(),
            Duration::from_millis(50),
            Some(state_dir.clone()),
            None,
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
            None,
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

    // ------------------------------------------------------------------
    // 入站媒体：语音转写 / 图片 CDN 下载解密
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn poll_once_maps_voice_transcription_as_text() {
        let server = wiremock::MockServer::start().await;
        mount_getupdates(
            &server,
            serde_json::json!({
                "ret": 0,
                "msgs": [
                    {
                        "from_user_id": "wxid_user@im.wechat",
                        "message_type": 1,
                        "context_token": "CTX-V",
                        "item_list": [ { "type": 3, "voice_item": { "text": " 明天提醒我开会 " } } ]
                    },
                    {
                        "from_user_id": "wxid_user@im.wechat",
                        "message_type": 1,
                        "context_token": "CTX-V2",
                        "item_list": [ { "type": 3, "voice_item": {} } ]
                    }
                ],
                "get_updates_buf": "CURSOR-V"
            }),
        )
            .await;

        let platform = platform_with_stub(server.uri());
        let events = platform.poll_once().await.expect("poll_once succeeds");

        assert_eq!(events.len(), 2);
        match &events[0] {
            Inbound::Message(message) => {
                assert_eq!(message.text, "[语音] 明天提醒我开会");
            }
            Inbound::Callback(_) => panic!("expected a message event"),
        }
        match &events[1] {
            Inbound::Message(message) => {
                assert_eq!(message.text, "[语音]（无转写文本）");
            }
            Inbound::Callback(_) => panic!("expected a message event"),
        }
    }

    /// AES-128-ECB + PKCS#7 的往返自测（用 Aes128 加密再解密，断言还原）。
    #[test]
    fn aes_ecb_roundtrip() {
        use aes_gcm::aes::cipher::common::Block;
        use aes_gcm::aes::cipher::{BlockCipherEncrypt, KeyInit};

        let key = [7u8; 16];
        let plain = b"hello wechat media!".to_vec();
        // PKCS#7 填充到 16 的整数倍。
        let pad = 16 - plain.len() % 16;
        let mut padded = plain.clone();
        padded.extend(std::iter::repeat(pad as u8).take(pad));

        let cipher = aes_gcm::aes::Aes128::new(Block::<aes_gcm::aes::Aes128>::from_slice(&key));
        let mut buf = padded.clone();
        for chunk in buf.chunks_exact_mut(16) {
            cipher.encrypt_block(Block::<aes_gcm::aes::Aes128>::from_mut_slice(chunk));
        }
        let decrypted = decrypt_aes_128_ecb(&key, &buf).expect("decrypt succeeds");
        assert_eq!(decrypted, plain);

        // 非块对齐 / 坏填充都要拒绝。
        assert!(decrypt_aes_128_ecb(&key, b"short").is_none());
        let mut bad = buf.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0xFF; // 破坏填充字节
        assert!(decrypt_aes_128_ecb(&key, &bad).is_none());
    }

    #[test]
    fn normalize_media_key_accepts_hex_and_base64_forms() {
        // hex 字段：32 个 hex 字符。
        let hex_key = normalize_media_key(Some("0f1e2d3c4b5a69788796a5b4c3d2e1f0"), None);
        assert_eq!(hex_key, Some([0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2, 0xe1, 0xf0]));
        // base64 字段：16 字节密钥。
        let b64 = base64::engine::general_purpose::STANDARD.encode([1u8; 16]);
        assert_eq!(
            normalize_media_key(None, Some(&b64)),
            Some([1u8; 16])
        );
        // base64 字段装的是 32 字符 hex ASCII（cc-connect 兼容形态）。
        let b64_hex = base64::engine::general_purpose::STANDARD
            .encode(b"0f1e2d3c4b5a69788796a5b4c3d2e1f0");
        assert!(normalize_media_key(None, Some(&b64_hex)).is_some());
        assert_eq!(normalize_media_key(None, Some("!!!!")), None);
        assert_eq!(normalize_media_key(None, None), None);
    }

    /// 完整图片链路：CDN 桩返回 ECB 密文 → 适配器下载解密落盘 → 消息文本
    /// 带上 `[图片] <路径>`，路径真实存在且内容即明文 PNG。
    #[tokio::test]
    async fn poll_once_downloads_decrypts_and_saves_inbound_images() {
        use aes_gcm::aes::cipher::common::Block;
        use aes_gcm::aes::cipher::{BlockCipherEncrypt, KeyInit};

        // 明文：最小 PNG 魔数 + 内容。
        let mut plain = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        plain.extend_from_slice(b"fake-png-body");
        let pad = 16 - plain.len() % 16;
        let mut padded = plain.clone();
        padded.extend(std::iter::repeat(pad as u8).take(pad));
        let key = [42u8; 16];
        let cipher = aes_gcm::aes::Aes128::new(Block::<aes_gcm::aes::Aes128>::from_slice(&key));
        let mut encrypted = padded;
        for chunk in encrypted.chunks_exact_mut(16) {
            cipher.encrypt_block(Block::<aes_gcm::aes::Aes128>::from_mut_slice(chunk));
        }

        let cdn = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/download"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_bytes(encrypted.clone()),
            )
            .mount(&cdn)
            .await;

        let api = wiremock::MockServer::start().await;
        mount_getupdates(
            &api,
            serde_json::json!({
                "ret": 0,
                "msgs": [
                    {
                        "from_user_id": "wxid_user@im.wechat",
                        "message_type": 1,
                        "context_token": "CTX-IMG",
                        "item_list": [
                            {
                                "type": 2,
                                "image_item": {
                                    "media": {
                                        "encrypt_query_param": "enc-param/with+special=&chars",
                                        "aes_key": base64::engine::general_purpose::STANDARD.encode(key)
                                    }
                                }
                            }
                        ]
                    }
                ],
                "get_updates_buf": "CURSOR-IMG"
            }),
        )
            .await;

        let state_dir = std::env::temp_dir().join(format!(
            "bamboo-wechat-img-test-{}",
            uuid::Uuid::new_v4()
        ));
        let platform = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            api.uri(),
            Duration::from_millis(50),
            Some(state_dir.clone()),
            None,
        )
        .with_cdn_base(cdn.uri());

        let events = platform.poll_once().await.expect("poll_once succeeds");
        assert_eq!(events.len(), 1, "a pure-image message must map");
        match &events[0] {
            Inbound::Message(message) => {
                assert!(message.text.starts_with("[图片] "), "text was: {}", message.text);
                let path = message.text.trim_start_matches("[图片] ").trim();
                let saved = std::fs::read(path).expect("image file exists on disk");
                assert_eq!(saved, plain, "decrypted bytes must match the plaintext");
                // CDN 请求确实带上了编码后的查询参数。
                let requests = wait_for_requests(&cdn, 1).await;
                assert!(requests[0]
                    .url
                    .query()
                    .unwrap_or_default()
                    .contains("encrypted_query_param=enc-param%2Fwith%2Bspecial%3D%26chars"));
            }
            Inbound::Callback(_) => panic!("expected a message event"),
        }

        let _ = std::fs::remove_dir_all(state_dir);
    }

    // ------------------------------------------------------------------
    // 出站附件：[SEND_FILE:] 标记 + iLink 上传投递
    // ------------------------------------------------------------------

    #[test]
    fn send_file_markers_are_extracted_and_stripped() {
        let text = "这是报告：\n[SEND_FILE: C:\\reports\\季报.docx]\n[SEND_FILE: /tmp/a.png]\n请查收。";
        let (visible, files) = extract_send_file_markers(text);
        assert_eq!(visible, "这是报告：\n请查收。");
        assert_eq!(
            files,
            vec![
                PathBuf::from("C:\\reports\\季报.docx"),
                PathBuf::from("/tmp/a.png")
            ]
        );

        // 没有标记时原样返回。
        let (visible, files) = extract_send_file_markers("普通回复");
        assert_eq!(visible, "普通回复");
        assert!(files.is_empty());

        // 半截标记（无右括号/空路径）不算标记，原样保留。
        let (visible, files) = extract_send_file_markers("[SEND_FILE: 没闭合");
        assert!(files.is_empty());
        assert!(visible.contains("[SEND_FILE:"));
    }

    /// 完整出站投递链路：getuploadurl → CDN POST（响应头带
    /// x-encrypted-param）→ sendmessage 携带 file_item（密钥为 base64(hex)、
    /// len 为明文长度）。
    #[tokio::test]
    async fn reply_delivers_send_file_markers_via_cdn_upload() {
        let state_dir = std::env::temp_dir().join(format!(
            "bamboo-wechat-outbound-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&state_dir).unwrap();
        let docx = state_dir.join("报告.docx");
        std::fs::write(&docx, b"PK\x03\x04 fake-docx-bytes").unwrap();

        let api = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/ilink/bot/getuploadurl"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ret": 0, "errcode": 0,
                    "upload_param": "UP-PARAM-1"
                })),
            )
            .mount(&api)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/ilink/bot/sendmessage"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "ret": 0, "errcode": 0 })),
            )
            .mount(&api)
            .await;

        let cdn = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/upload"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .insert_header("x-encrypted-param", "DL-PARAM-9"),
            )
            .mount(&cdn)
            .await;

        let platform = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            api.uri(),
            Duration::from_millis(50),
            Some(state_dir.clone()),
            None,
        )
        .with_cdn_base(cdn.uri());

        let ctx = ReplyCtx(serde_json::json!({
            "to_user_id": "wxid_user@im.wechat",
            "context_token": "CTX-F"
        }));
        platform
            .reply(
                &ctx,
                OutboundMessage::text(format!(
                    "报告来了\n[SEND_FILE: {}]",
                    docx.to_string_lossy()
                )),
            )
            .await
            .expect("reply succeeds");

        // sendmessage 收到两条：文本 + file_item。
        let requests = wait_for_requests(&api, 2).await;
        let file_msg = requests
            .iter()
            .map(body_json)
            .find(|body| body.pointer("/msg/item_list/0/file_item").is_some())
            .expect("one sendmessage carries a file_item");
        let file_item = file_msg.pointer("/msg/item_list/0/file_item").unwrap();
        assert_eq!(
            file_item.pointer("/file_name").and_then(|v| v.as_str()),
            Some("报告.docx")
        );
        assert_eq!(
            file_item.pointer("/len").and_then(|v| v.as_str()),
            Some("20".to_string()).as_deref()
        );
        let media = file_item.pointer("/media").unwrap();
        assert_eq!(
            media.pointer("/encrypt_query_param").and_then(|v| v.as_str()),
            Some("DL-PARAM-9")
        );
        assert_eq!(
            media.pointer("/encrypt_type").and_then(|v| v.as_i64()),
            Some(1)
        );
        // aes_key 形态：base64(32 字符 hex)。
        let aes_key_b64 = media.pointer("/aes_key").and_then(|v| v.as_str()).unwrap();
        let aes_key_hex = base64::engine::general_purpose::STANDARD
            .decode(aes_key_b64)
            .expect("base64");
        let aes_key_hex = String::from_utf8(aes_key_hex).expect("hex ascii");
        assert_eq!(aes_key_hex.len(), 32);
        assert!(aes_key_hex.bytes().all(|b| b.is_ascii_hexdigit()));

        // 上传请求：URL 带 upload_param 与 filekey；请求体是 16 字节对齐的密文。
        let upload_requests = wait_for_requests(&cdn, 1).await;
        let query = upload_requests[0].url.query().unwrap_or_default();
        assert!(query.contains("encrypted_query_param=UP-PARAM-1"));
        assert!(query.contains("filekey="));
        let body_len = upload_requests[0].body.len();
        assert!(body_len % 16 == 0 && body_len >= 20);

        let _ = std::fs::remove_dir_all(state_dir);
    }

    // ------------------------------------------------------------------
    // 出站语音：[SEND_VOICE:] 标记 + mirror/always 决策 + 失败回退
    // ------------------------------------------------------------------

    fn test_voice_config(
        tts_base_url: String,
        reply_mode: wechat_voice::VoiceReplyMode,
        file_asr: bool,
    ) -> VoiceConfig {
        VoiceConfig {
            api_key: "sk-test".to_string(),
            base_url: tts_base_url,
            tts_model: "FunAudioLLM/CosyVoice2-0.5B".to_string(),
            tts_voice: "FunAudioLLM/CosyVoice2-0.5B:anna".to_string(),
            sample_rate: 16_000,
            speed: 1.0,
            bitrate: 25_000,
            reply_mode,
            delivery: wechat_voice::VoiceDelivery::Bubble,
            asr_model: "FunAudioLLM/SenseVoiceSmall".to_string(),
            file_asr,
        }
    }

    fn reply_ctx_with(had_voice: bool) -> ReplyCtx {
        ReplyCtx(serde_json::json!({
            "to_user_id": "wxid_user@im.wechat",
            "context_token": "CTX-V",
            "had_voice": had_voice,
        }))
    }

    async fn mount_siliconflow_tts(server: &wiremock::MockServer, pcm_bytes: usize) {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/audio/speech"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_bytes(vec![0u8; pcm_bytes]),
            )
            .mount(server)
            .await;
    }

    async fn mount_ilink_send_stack(api: &wiremock::MockServer) {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/ilink/bot/getuploadurl"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "ret": 0, "errcode": 0, "upload_param": "UP-VOICE-1"
                })),
            )
            .mount(api)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/ilink/bot/sendmessage"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "ret": 0, "errcode": 0 })),
            )
            .mount(api)
            .await;
    }

    async fn mount_cdn_upload(cdn: &wiremock::MockServer) {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/upload"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .insert_header("x-encrypted-param", "DL-VOICE-9"),
            )
            .mount(cdn)
            .await;
    }

    #[test]
    fn send_voice_markers_are_extracted_and_stripped() {
        let text = "说明：\n[SEND_VOICE: 明天下午三点开会]\n以上。";
        let (visible, voices) = extract_send_voice_markers(text);
        assert_eq!(visible, "说明：\n以上。");
        assert_eq!(voices, vec!["明天下午三点开会".to_string()]);

        let (visible, voices) = extract_send_voice_markers("普通回复");
        assert_eq!(visible, "普通回复");
        assert!(voices.is_empty());

        // 半截标记原样保留。
        let (visible, voices) = extract_send_voice_markers("[SEND_VOICE: 没闭合");
        assert!(voices.is_empty());
        assert!(visible.contains("[SEND_VOICE:"));
    }

    /// 未配置语音时标记文本并入正文——模型误用标记也不丢内容、不报错。
    #[tokio::test]
    async fn reply_voice_marker_without_config_keeps_text() {
        let api = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/ilink/bot/sendmessage"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "ret": 0, "errcode": 0 })),
            )
            .mount(&api)
            .await;

        let platform = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            api.uri(),
            Duration::from_millis(10),
            None,
            None,
        );
        platform
            .reply(
                &reply_ctx_with(false),
                OutboundMessage::text("结论如上\n[SEND_VOICE: 本应念出来的部分]"),
            )
            .await
            .expect("reply succeeds");

        let requests = wait_for_requests(&api, 1).await;
        let sent = body_json(&requests[0]);
        let text = sent
            .pointer("/msg/item_list/0/text_item/text")
            .and_then(|value| value.as_str())
            .unwrap();
        assert!(text.contains("结论如上"));
        assert!(text.contains("本应念出来的部分"));
    }

    /// 完整语音投递链路：TTS(pcm) → SILK 编码 → CDN 上传(FILE 通道) →
    /// sendmessage voice_item（极简 media + encode_type）。
    #[tokio::test]
    async fn reply_delivers_voice_marker_as_voice_item() {
        let tts = wiremock::MockServer::start().await;
        mount_siliconflow_tts(&tts, 16_000 * 2).await; // 1 秒 16kHz 静音。
        let api = wiremock::MockServer::start().await;
        mount_ilink_send_stack(&api).await;
        let cdn = wiremock::MockServer::start().await;
        mount_cdn_upload(&cdn).await;

        let platform = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            api.uri(),
            Duration::from_millis(10),
            None,
            Some(test_voice_config(
                tts.uri(),
                wechat_voice::VoiceReplyMode::Off,
                false,
            )),
        )
        .with_cdn_base(cdn.uri());

        platform
            .reply(
                &reply_ctx_with(false),
                OutboundMessage::text("补充说明\n[SEND_VOICE: 明天下午三点开会，记得带笔记本]"),
            )
            .await
            .expect("reply succeeds");

        // TTS 请求体：pcm@16000 + 目标文本。
        let tts_requests = wait_for_requests(&tts, 1).await;
        let tts_body = body_json(&tts_requests[0]);
        assert_eq!(
            tts_body.get("response_format").and_then(|v| v.as_str()),
            Some("pcm")
        );
        assert_eq!(
            tts_body.get("sample_rate").and_then(|v| v.as_i64()),
            Some(16_000)
        );
        assert_eq!(
            tts_body.get("input").and_then(|v| v.as_str()),
            Some("明天下午三点开会，记得带笔记本")
        );

        // getuploadurl：语音按 FILE 通道上传（VOICE=4 真机静默丢弃，
        // 对齐 cc-connect 真机可用配方）。
        let api_requests = wait_for_requests(&api, 2).await;
        let upload_req = api_requests
            .iter()
            .find(|request| request.url.path().contains("getuploadurl"))
            .expect("getuploadurl called");
        assert_eq!(
            body_json(upload_req).get("media_type").and_then(|v| v.as_i64()),
            Some(UPLOAD_MEDIA_FILE)
        );

        // sendmessage：文本（不含标记）+ voice_item（字段形状对齐入站取证：
        // encode_type + sample_rate + bits_per_sample + playtime 毫秒）。
        let voice_msg = api_requests
            .iter()
            .map(body_json)
            .find(|body| body.pointer("/msg/item_list/0/voice_item").is_some())
            .expect("one sendmessage carries a voice_item");
        let voice_item = voice_msg.pointer("/msg/item_list/0/voice_item").unwrap();
        assert_eq!(
            voice_item.pointer("/encode_type").and_then(|v| v.as_i64()),
            Some(VOICE_ENCODE_TYPE_SILK),
            "no inbound voice observed in this test → default fallback"
        );
        assert_eq!(
            voice_item.pointer("/sample_rate").and_then(|v| v.as_i64()),
            Some(16_000)
        );
        assert_eq!(
            voice_item.pointer("/bits_per_sample").and_then(|v| v.as_i64()),
            Some(16)
        );
        assert_eq!(
            voice_item.pointer("/playtime").and_then(|v| v.as_i64()),
            Some(1000),
            "1s of 16kHz pcm → 1000ms"
        );
        assert_eq!(
            voice_item
                .pointer("/media/encrypt_query_param")
                .and_then(|v| v.as_str()),
            Some("DL-VOICE-9")
        );
        let text_msg = api_requests
            .iter()
            .map(body_json)
            .find(|body| body.pointer("/msg/item_list/0/text_item").is_some())
            .expect("visible text was sent");
        assert_eq!(
            text_msg
                .pointer("/msg/item_list/0/text_item/text")
                .and_then(|v| v.as_str()),
            Some("补充说明")
        );

        // CDN 上传体解密后是 \x02#!SILK_V3 腾讯变体（上传的是 AES 密文，
        // 密钥在 getuploadurl 请求的 aeskey hex 字段里）。
        let cdn_requests = wait_for_requests(&cdn, 1).await;
        let upload_req = api_requests
            .iter()
            .find(|request| request.url.path().contains("getuploadurl"))
            .expect("getuploadurl called");
        let aeskey_hex = body_json(upload_req)
            .get("aeskey")
            .and_then(|value| value.as_str())
            .expect("aeskey present")
            .to_string();
        let key_bytes: [u8; 16] = (0..16)
            .map(|index| u8::from_str_radix(&aeskey_hex[index * 2..index * 2 + 2], 16).unwrap())
            .collect::<Vec<u8>>()
            .try_into()
            .unwrap();
        let decrypted = decrypt_aes_128_ecb(&key_bytes, &cdn_requests[0].body)
            .expect("uploaded ciphertext decrypts");
        assert!(decrypted.starts_with(b"\x02#!SILK_V3"));
    }

    /// encode_type 自动校准：入站语音条携带的 encode_type 是微信自己的
    /// SILK 语音在本线格式下的真实取值——出站语音直接采用该观测值，
    /// 覆盖官方包注释（6=silk）与 cc-connect 注释（1=SILK）的矛盾兜底。
    #[tokio::test]
    async fn outbound_voice_encode_type_adopts_observed_inbound_value() {
        let inbound = wiremock::MockServer::start().await;
        mount_getupdates(
            &inbound,
            serde_json::json!({
                "ret": 0,
                "msgs": [ {
                    "from_user_id": "wxid_user@im.wechat",
                    "message_type": 1,
                    "context_token": "CTX-OBS",
                    "item_list": [ { "type": 3, "voice_item": {
                        "text": " 观测用的入站语音 ",
                        "encode_type": 1
                    } } ]
                } ],
                "get_updates_buf": "CURSOR-OBS"
            }),
        )
        .await;
        let platform = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            inbound.uri(),
            Duration::from_millis(10),
            None,
            None,
        );
        platform.poll_once().await.expect("poll_once succeeds");
        assert_eq!(
            platform.observed_voice_encode_type.load(Ordering::Relaxed),
            1,
            "inbound voice encode_type must be observed"
        );

        // 出站语音采用观测值而非兜底常量。
        let tts = wiremock::MockServer::start().await;
        mount_siliconflow_tts(&tts, 16_000 * 2).await;
        let api = wiremock::MockServer::start().await;
        mount_ilink_send_stack(&api).await;
        let cdn = wiremock::MockServer::start().await;
        mount_cdn_upload(&cdn).await;
        let sender = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            api.uri(),
            Duration::from_millis(10),
            None,
            Some(test_voice_config(
                tts.uri(),
                wechat_voice::VoiceReplyMode::Off,
                false,
            )),
        )
        .with_cdn_base(cdn.uri());
        sender.observed_voice_encode_type.store(1, Ordering::Relaxed);

        sender
            .reply(
                &reply_ctx_with(false),
                OutboundMessage::text("[SEND_VOICE: 校准测试]"),
            )
            .await
            .expect("reply succeeds");

        let api_requests = wait_for_requests(&api, 2).await;
        let voice_msg = api_requests
            .iter()
            .map(body_json)
            .find(|body| body.pointer("/msg/item_list/0/voice_item").is_some())
            .expect("one sendmessage carries a voice_item");
        assert_eq!(
            voice_msg
                .pointer("/msg/item_list/0/voice_item/encode_type")
                .and_then(|v| v.as_i64()),
            Some(1),
            "outbound must adopt the observed inbound encode_type, not the fallback"
        );
    }

    /// 默认 `file` 投递：TTS 直接合成 mp3（response_format=mp3、44100）→
    /// FILE 通道上传 → file_item（不构造 voice_item——iLink 不渲染）。
    #[tokio::test]
    async fn default_delivery_sends_voice_as_mp3_file() {
        let tts = wiremock::MockServer::start().await;
        mount_siliconflow_tts(&tts, 4096).await; // 任意二进制即可（桩不校验格式）。
        let api = wiremock::MockServer::start().await;
        mount_ilink_send_stack(&api).await;
        let cdn = wiremock::MockServer::start().await;
        mount_cdn_upload(&cdn).await;

        let mut voice = test_voice_config(
            tts.uri(),
            wechat_voice::VoiceReplyMode::Off,
            false,
        );
        voice.delivery = wechat_voice::VoiceDelivery::File;
        let platform = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            api.uri(),
            Duration::from_millis(10),
            None,
            Some(voice),
        )
        .with_cdn_base(cdn.uri());

        platform
            .reply(
                &reply_ctx_with(false),
                OutboundMessage::text("[SEND_VOICE: 以文件形态发送的语音回复]"),
            )
            .await
            .expect("reply succeeds");

        // TTS 请求 mp3 + 44100（mp3 档位与 pcm 不同）。
        let tts_requests = wait_for_requests(&tts, 1).await;
        let tts_body = body_json(&tts_requests[0]);
        assert_eq!(
            tts_body.get("response_format").and_then(|v| v.as_str()),
            Some("mp3")
        );
        assert_eq!(
            tts_body.get("sample_rate").and_then(|v| v.as_i64()),
            Some(44_100)
        );

        let api_requests = wait_for_requests(&api, 2).await;
        // FILE 通道上传。
        let upload_req = api_requests
            .iter()
            .find(|request| request.url.path().contains("getuploadurl"))
            .expect("getuploadurl called");
        assert_eq!(
            body_json(upload_req).get("media_type").and_then(|v| v.as_i64()),
            Some(UPLOAD_MEDIA_FILE)
        );
        // sendmessage 携带 file_item（mp3 文件名 + len 字符串）。
        let file_msg = api_requests
            .iter()
            .map(body_json)
            .find(|body| body.pointer("/msg/item_list/0/file_item").is_some())
            .expect("one sendmessage carries a file_item");
        let file_item = file_msg.pointer("/msg/item_list/0/file_item").unwrap();
        let name = file_item
            .pointer("/file_name")
            .and_then(|v| v.as_str())
            .expect("file_name present");
        assert!(name.starts_with("语音回复-") && name.ends_with(".mp3"), "{name}");
        assert_eq!(
            file_item.pointer("/len").and_then(|v| v.as_str()),
            Some("4096")
        );
        assert!(
            file_msg
                .pointer("/msg/item_list/0/voice_item")
                .is_none(),
            "file delivery must not construct a voice_item"
        );
    }

    /// mirror 决策：语音入站 → 自动转语音（正文不再重复发文字）；
    /// 文字入站或内容不适合（代码）→ 保持文字，不调用 TTS。
    #[tokio::test]
    async fn reply_mirror_mode_decision_rules() {
        // 案例 A：mirror + 语音入站 + 适合内容 → 语音条、无文本条。
        {
            let tts = wiremock::MockServer::start().await;
            mount_siliconflow_tts(&tts, 16_000 * 2).await;
            let api = wiremock::MockServer::start().await;
            mount_ilink_send_stack(&api).await;
            let cdn = wiremock::MockServer::start().await;
            mount_cdn_upload(&cdn).await;
            let platform = WechatPlatform::with_options(
                TEST_TOKEN.to_string(),
                api.uri(),
                Duration::from_millis(10),
                None,
                Some(test_voice_config(
                    tts.uri(),
                    wechat_voice::VoiceReplyMode::Mirror,
                    false,
                )),
            )
            .with_cdn_base(cdn.uri());
            platform
                .reply(&reply_ctx_with(true), OutboundMessage::text("好的，明天见"))
                .await
                .expect("reply succeeds");

            let api_requests = wait_for_requests(&api, 2).await;
            for request in &api_requests {
                if !request.url.path().contains("sendmessage") {
                    continue;
                }
                let body = body_json(request);
                assert!(
                    body.pointer("/msg/item_list/0/voice_item").is_some(),
                    "mirror + voice inbound must send voice only, got: {body}"
                );
            }
        }

        // 案例 B：mirror + 文字入站 → 只发文本，不调 TTS。
        {
            let tts = wiremock::MockServer::start().await;
            let api = wiremock::MockServer::start().await;
            mount_ilink_send_stack(&api).await;
            let cdn = wiremock::MockServer::start().await;
            mount_cdn_upload(&cdn).await;
            let platform = WechatPlatform::with_options(
                TEST_TOKEN.to_string(),
                api.uri(),
                Duration::from_millis(10),
                None,
                Some(test_voice_config(
                    tts.uri(),
                    wechat_voice::VoiceReplyMode::Mirror,
                    false,
                )),
            )
            .with_cdn_base(cdn.uri());
            platform
                .reply(&reply_ctx_with(false), OutboundMessage::text("好的，明天见"))
                .await
                .expect("reply succeeds");

            let api_requests = wait_for_requests(&api, 1).await;
            let body = body_json(&api_requests[0]);
            assert!(body.pointer("/msg/item_list/0/text_item").is_some());
            assert!(tts.received_requests().await.unwrap_or_default().is_empty());
        }

        // 案例 C：mirror + 语音入站 + 代码内容 → 降级文本，不调 TTS。
        {
            let tts = wiremock::MockServer::start().await;
            let api = wiremock::MockServer::start().await;
            mount_ilink_send_stack(&api).await;
            let cdn = wiremock::MockServer::start().await;
            mount_cdn_upload(&cdn).await;
            let platform = WechatPlatform::with_options(
                TEST_TOKEN.to_string(),
                api.uri(),
                Duration::from_millis(10),
                None,
                Some(test_voice_config(
                    tts.uri(),
                    wechat_voice::VoiceReplyMode::Mirror,
                    false,
                )),
            )
            .with_cdn_base(cdn.uri());
            platform
                .reply(
                    &reply_ctx_with(true),
                    OutboundMessage::text("运行 `cargo build` 后告诉我结果"),
                )
                .await
                .expect("reply succeeds");

            let api_requests = wait_for_requests(&api, 1).await;
            let body = body_json(&api_requests[0]);
            assert!(body.pointer("/msg/item_list/0/text_item").is_some());
            assert!(tts.received_requests().await.unwrap_or_default().is_empty());
        }
    }

    /// TTS 失败回退文字：自动模式原文尚未发过，回退把原文带上。
    #[tokio::test]
    async fn voice_synthesis_failure_falls_back_to_text() {
        let tts = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/audio/speech"))
            .respond_with(wiremock::ResponseTemplate::new(500).set_body_json(
                serde_json::json!({ "message": "额度不足" }),
            ))
            .mount(&tts)
            .await;
        let api = wiremock::MockServer::start().await;
        mount_ilink_send_stack(&api).await;
        let cdn = wiremock::MockServer::start().await;
        mount_cdn_upload(&cdn).await;

        let platform = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            api.uri(),
            Duration::from_millis(10),
            None,
            Some(test_voice_config(
                tts.uri(),
                wechat_voice::VoiceReplyMode::Always,
                false,
            )),
        )
        .with_cdn_base(cdn.uri());

        platform
            .reply(&reply_ctx_with(false), OutboundMessage::text("好的，明天见"))
            .await
            .expect("reply succeeds");

        let api_requests = wait_for_requests(&api, 1).await;
        let body = body_json(&api_requests[0]);
        let text = body
            .pointer("/msg/item_list/0/text_item/text")
            .and_then(|value| value.as_str())
            .unwrap();
        assert!(text.contains("语音合成失败"), "got: {text}");
        assert!(text.contains("额度不足"), "got: {text}");
        assert!(text.contains("好的，明天见"), "got: {text}");
    }

    // ------------------------------------------------------------------
    // 入站音频文件转写（file_asr 开关）
    // ------------------------------------------------------------------

    async fn mount_siliconflow_asr(server: &wiremock::MockServer, text: &str) {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/audio/transcriptions"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "text": text })),
            )
            .mount(server)
            .await;
    }

    async fn run_audio_file_asr_case(file_asr: bool) -> (Vec<Inbound>, wiremock::MockServer) {
        let key: [u8; 16] = rand::random();
        let plain = b"fake-mp3-bytes".to_vec();
        let encrypted = encrypt_aes_128_ecb(&key, &plain);

        let cdn = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/download"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_bytes(encrypted.clone()),
            )
            .mount(&cdn)
            .await;

        let api = wiremock::MockServer::start().await;
        mount_getupdates(
            &api,
            serde_json::json!({
                "ret": 0,
                "msgs": [
                    {
                        "from_user_id": "wxid_user@im.wechat",
                        "message_type": 1,
                        "context_token": "CTX-AUDIO",
                        "item_list": [
                            {
                                "type": 4,
                                "file_item": {
                                    "media": {
                                        "encrypt_query_param": "audio-param",
                                        "aes_key": base64::engine::general_purpose::STANDARD.encode(key)
                                    },
                                    "file_name": "meeting.mp3"
                                }
                            }
                        ]
                    }
                ],
                "get_updates_buf": "CURSOR-AUDIO"
            }),
        )
        .await;

        let siliconflow = wiremock::MockServer::start().await;
        mount_siliconflow_asr(&siliconflow, " 会议纪要内容 ").await;

        let state_dir = std::env::temp_dir().join(format!(
            "bamboo-wechat-asr-test-{}",
            uuid::Uuid::new_v4()
        ));
        let platform = WechatPlatform::with_options(
            TEST_TOKEN.to_string(),
            api.uri(),
            Duration::from_millis(10),
            Some(state_dir.clone()),
            Some(test_voice_config(
                siliconflow.uri(),
                wechat_voice::VoiceReplyMode::Off,
                file_asr,
            )),
        )
        .with_cdn_base(cdn.uri());

        let events = platform.poll_once().await.expect("poll_once succeeds");
        let _ = std::fs::remove_dir_all(state_dir);
        (events, siliconflow)
    }

    #[tokio::test]
    async fn poll_once_transcribes_audio_files_when_enabled() {
        let (events, siliconflow) = run_audio_file_asr_case(true).await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            Inbound::Message(message) => {
                assert!(message.text.contains("[文件] "), "got: {}", message.text);
                assert!(message.text.contains("meeting.mp3"), "got: {}", message.text);
                assert!(
                    message.text.contains("[音频转写] 会议纪要内容"),
                    "got: {}",
                    message.text
                );
            }
            Inbound::Callback(_) => panic!("expected a message event"),
        }
        // 转写确实上传了音频。
        let requests = wait_for_requests(&siliconflow, 1).await;
        let content_type = requests[0]
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(content_type.starts_with("multipart/form-data"), "got: {content_type}");
        assert!(requests[0]
            .body
            .windows(b"meeting.mp3".len())
            .any(|window| window == b"meeting.mp3"));
    }

    #[tokio::test]
    async fn poll_once_skips_audio_transcription_when_disabled() {
        let (events, siliconflow) = run_audio_file_asr_case(false).await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            Inbound::Message(message) => {
                assert!(message.text.contains("[文件] "));
                assert!(!message.text.contains("[音频转写]"));
            }
            Inbound::Callback(_) => panic!("expected a message event"),
        }
        assert!(siliconflow
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty());
    }
}
