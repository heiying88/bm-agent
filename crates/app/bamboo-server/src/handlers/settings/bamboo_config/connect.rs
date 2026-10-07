use actix_web::{web, HttpResponse};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    app_state::AppState,
    error::AppError,
    handlers::settings::credential_action::{credential_status_view, CredentialAction},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectMutationRequest {
    expected_revision: u64,
    data: ConnectMutationData,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectMutationData {
    platforms: Vec<ConnectPlatformMutation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectPlatformMutation {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    project_id: Option<bamboo_domain::ProjectId>,
    #[serde(rename = "type")]
    platform_type: String,
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    allow_from: Vec<String>,
    #[serde(default)]
    admin_from: Vec<String>,
    /// 微信语音能力的非秘密部分（`voice` 段：模型/音色/开关等，原样
    /// 透传）。**缺省/null = 保留现有配置**——不认识该字段的旧客户端
    /// 整单保存时不会把它抹掉；提交即整体替换。语音 API 密钥是平台级
    /// secret，走 `voice_api_key_change`（Keep/Replace/Clear），与
    /// `token_change` 同一套凭据管道。
    #[serde(default)]
    voice: Option<bamboo_config::WechatVoiceConfig>,
    /// 微信网关 Project Dream 自动运行计划（`dream` 段）。**缺省/null =
    /// 保留现有配置**；提交即整体替换（校验 mode/daily_at/idle_minutes）。
    #[serde(default)]
    dream: Option<bamboo_config::WechatDreamConfig>,
    /// 微信网关首条消息自动项目（`auto_project` 段）。**缺省/null = 保留
    /// 现有配置**；提交即整体替换。
    #[serde(default)]
    auto_project: Option<bamboo_config::WechatAutoProjectConfig>,
    /// "信任首个发信人"闩锁（v0.0.4）。**缺省/null = 保留现有值**；
    /// 扫码登录端点在未显式提供 allow_from 时默认置 true。
    #[serde(default)]
    trust_first_sender: Option<bool>,
    /// 微信"正在输入中"指示。**缺省/null = 保留现有值**。
    #[serde(default)]
    typing_indicator: Option<bool>,
    #[serde(default)]
    token_change: Option<CredentialAction>,
    #[serde(default)]
    app_secret_change: Option<CredentialAction>,
    #[serde(default)]
    voice_api_key_change: Option<CredentialAction>,
}

fn existing_platform<'a>(
    current: &'a bamboo_config::ConnectConfig,
    candidate: &ConnectPlatformMutation,
    index: usize,
) -> Option<&'a bamboo_config::ConnectPlatformConfig> {
    candidate
        .id
        .as_deref()
        .and_then(|id| {
            current
                .platforms
                .iter()
                .find(|platform| platform.id.as_deref() == Some(id))
        })
        .or_else(|| {
            current
                .platforms
                .get(index)
                .filter(|platform| platform.platform_type == candidate.platform_type)
        })
        .or_else(|| {
            let mut matches = current
                .platforms
                .iter()
                .filter(|platform| platform.platform_type == candidate.platform_type);
            let first = matches.next();
            first.filter(|_| matches.next().is_none())
        })
}

fn apply_action(
    action: Option<&CredentialAction>,
    current_value: Option<String>,
    current_ref: Option<bamboo_config::CredentialRef>,
    current_configured: bool,
) -> (Option<String>, Option<bamboo_config::CredentialRef>, bool) {
    match action {
        None | Some(CredentialAction::Keep) => (current_value, current_ref, current_configured),
        Some(CredentialAction::Replace { value }) => (Some(value.clone()), current_ref, true),
        Some(CredentialAction::Clear) => (None, current_ref, false),
    }
}

fn connect_response(
    section: bamboo_config::SectionEnvelope<serde_json::Value>,
    config: &bamboo_config::Config,
    statuses: Vec<bamboo_config::CredentialStatus>,
    credential_health: bamboo_config::CredentialStoreHealth,
) -> serde_json::Value {
    let statuses = statuses
        .into_iter()
        .map(|status| (status.credential_ref.clone(), status))
        .collect::<BTreeMap<_, _>>();
    let active_refs = config
        .connect
        .platforms
        .iter()
        .flat_map(|platform| {
            [
                platform.token_credential_ref.clone(),
                platform.app_secret_credential_ref.clone(),
                platform.voice_api_key_credential_ref.clone(),
            ]
        })
        .flatten()
        .collect::<BTreeSet<_>>();
    let credentials = active_refs
        .into_iter()
        .map(|reference| {
            credential_status_view(
                Some(&reference),
                true,
                statuses.get(&reference),
                &credential_health,
            )
        })
        .collect::<Vec<_>>();
    let credential_status = config
        .connect
        .platforms
        .iter()
        .enumerate()
        .map(|(index, platform)| {
            let key = platform
                .id
                .clone()
                .unwrap_or_else(|| format!("index-{index}"));
            let token_status = platform
                .token_credential_ref
                .as_ref()
                .and_then(|reference| statuses.get(reference));
            let app_secret_status = platform
                .app_secret_credential_ref
                .as_ref()
                .and_then(|reference| statuses.get(reference));
            let voice_api_key_status = platform
                .voice_api_key_credential_ref
                .as_ref()
                .and_then(|reference| statuses.get(reference));
            (
                key,
                serde_json::json!({
                    "token": credential_status_view(
                        platform.token_credential_ref.as_ref(),
                        platform.token_configured,
                        token_status,
                        &credential_health,
                    ),
                    "app_secret": credential_status_view(
                        platform.app_secret_credential_ref.as_ref(),
                        platform.app_secret_configured,
                        app_secret_status,
                        &credential_health,
                    ),
                    "voice_api_key": credential_status_view(
                        platform.voice_api_key_credential_ref.as_ref(),
                        platform.voice_api_key_configured,
                        voice_api_key_status,
                        &credential_health,
                    ),
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    serde_json::json!({
        "revision": section.revision,
        "section": section,
        "credentials": credentials,
        "credential_status": credential_status,
        "credential_health": credential_health,
    })
}

/// Return the Connect section and one secret-free credential status snapshot.
pub async fn get_connect_config(app_state: web::Data<AppState>) -> Result<HttpResponse, AppError> {
    let exact = app_state
        .read_exact_credential_section(bamboo_config::SectionId::Connect)
        .await?;
    Ok(HttpResponse::Ok().json(connect_response(
        exact.section,
        &exact.config,
        exact.metadata.credential_statuses,
        exact.metadata.credential_health,
    )))
}

/// Replace Connect metadata and explicitly keep, replace, or clear platform
/// secrets using only the Connect section revision as the public CAS token.
pub async fn put_connect_config(
    app_state: web::Data<AppState>,
    payload: web::Json<ConnectMutationRequest>,
) -> Result<HttpResponse, AppError> {
    apply_connect_mutation(&app_state, payload.into_inner()).await
}

/// [`put_connect_config`] 的核心（扫码登录确认后的落库同样复用）。
pub(crate) async fn apply_connect_mutation(
    app_state: &AppState,
    payload: ConnectMutationRequest,
) -> Result<HttpResponse, AppError> {
    let mut ids = BTreeSet::new();
    let mut intents = bamboo_config::patch::ConnectSecretIntents::default();
    for (index, platform) in payload.data.platforms.iter().enumerate() {
        if platform
            .id
            .as_deref()
            .is_some_and(|id| id.trim().is_empty() || !ids.insert(id.to_string()))
        {
            return Err(AppError::BadRequest(
                "connect platform ids must be nonempty and unique".to_string(),
            ));
        }
        if platform.platform_type.trim().is_empty() {
            return Err(AppError::BadRequest(
                "connect platform type must be nonempty".to_string(),
            ));
        }
        // 只校验显式提交的 dream/auto_project；缺省时写入路径会保留的
        // 现有值在当初写入时已校验过。
        if platform.platform_type == "wechat" {
            if let Some(dream) = &platform.dream {
                if let Err(message) = dream.validate() {
                    return Err(AppError::BadRequest(format!(
                        "connect platform {index}: {message}"
                    )));
                }
            }
            if let Some(auto_project) = &platform.auto_project {
                if let Err(message) = auto_project.validate() {
                    return Err(AppError::BadRequest(format!(
                        "connect platform {index}: {message}"
                    )));
                }
            }
        }
        for (label, action, target) in [
            (
                "connect token",
                platform.token_change.as_ref(),
                &mut intents.token,
            ),
            (
                "connect app secret",
                platform.app_secret_change.as_ref(),
                &mut intents.app_secret,
            ),
            (
                "connect voice api key",
                platform.voice_api_key_change.as_ref(),
                &mut intents.voice_api_key,
            ),
        ] {
            if let Some(action) = action {
                action.validate(label)?;
                if !action.is_keep() {
                    target.insert(index);
                }
            }
        }
    }

    let expected_revision = payload.expected_revision;
    let data = payload.data;
    let (config, _, metadata, section) = app_state
        .update_connect_credentials(expected_revision, intents, move |config| {
            let current = config.connect.clone();
            let mut platforms = Vec::with_capacity(data.platforms.len());
            for (index, input) in data.platforms.into_iter().enumerate() {
                let existing = existing_platform(&current, &input, index);
                let (token, token_credential_ref, token_configured) = apply_action(
                    input.token_change.as_ref(),
                    existing.and_then(|platform| platform.token.clone()),
                    existing.and_then(|platform| platform.token_credential_ref.clone()),
                    existing.is_some_and(|platform| platform.token_configured),
                );
                let (app_secret, app_secret_credential_ref, app_secret_configured) = apply_action(
                    input.app_secret_change.as_ref(),
                    existing.and_then(|platform| platform.app_secret.clone()),
                    existing.and_then(|platform| platform.app_secret_credential_ref.clone()),
                    existing.is_some_and(|platform| platform.app_secret_configured),
                );
                let (voice_api_key, voice_api_key_credential_ref, voice_api_key_configured) =
                    apply_action(
                        input.voice_api_key_change.as_ref(),
                        existing.and_then(|platform| platform.voice_api_key.clone()),
                        existing.and_then(|platform| platform.voice_api_key_credential_ref.clone()),
                        existing.is_some_and(|platform| platform.voice_api_key_configured),
                    );
                platforms.push(bamboo_config::ConnectPlatformConfig {
                    // voice 缺省/null 保留现有值（见字段注释）；提交则整体替换。
                    voice: input
                        .voice
                        .or_else(|| existing.and_then(|platform| platform.voice.clone())),
                    // dream / auto_project 同 voice 的保留语义。
                    dream: input
                        .dream
                        .or_else(|| existing.and_then(|platform| platform.dream.clone())),
                    auto_project: input
                        .auto_project
                        .or_else(|| existing.and_then(|platform| platform.auto_project.clone())),
                    id: input.id,
                    project_id: input.project_id,
                    platform_type: input.platform_type,
                    token,
                    token_encrypted: None,
                    token_credential_ref,
                    token_configured,
                    app_id: input.app_id,
                    app_secret,
                    app_secret_encrypted: None,
                    app_secret_credential_ref,
                    app_secret_configured,
                    voice_api_key,
                    voice_api_key_encrypted: None,
                    voice_api_key_credential_ref,
                    voice_api_key_configured,
                    domain: input.domain,
                    trust_first_sender: input
                        .trust_first_sender
                        .unwrap_or_else(|| existing.is_some_and(|p| p.trust_first_sender)),
                    typing_indicator: input
                        .typing_indicator
                        .unwrap_or_else(|| existing.is_some_and(|p| p.typing_indicator)),
                    allow_from: input.allow_from,
                    admin_from: input.admin_from,
                });
            }
            config.connect = bamboo_config::ConnectConfig { platforms };
            Ok(())
        })
        .await?;
    let section = section.ok_or_else(|| {
        AppError::InternalError(anyhow::anyhow!(
            "connect mutation completed without a typed section envelope"
        ))
    })?;
    Ok(HttpResponse::Ok().json(connect_response(
        section,
        &config,
        metadata.credential_statuses,
        metadata.credential_health,
    )))
}

// ---------------------------------------------------------------------------
// 微信扫码登录（设置页流程）：iLink 网关无 CORS 头，浏览器无法直读，
// 由本服务同源转发；确认后的 bot_token 只驻留短 TTL 内存缓存，经
// `apply` 走与手填 token 完全相同的凭据管道落库，不经浏览器明文。
// ---------------------------------------------------------------------------

/// 扫码确认结果的服务器侧暂存（qrcode_id → 新 token），5 分钟 TTL。
static QR_CONFIRMED_TOKENS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, (String, std::time::Instant)>>,
> = std::sync::OnceLock::new();

const QR_TOKEN_TTL: std::time::Duration = std::time::Duration::from_secs(300);

fn qr_token_cache(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, (String, std::time::Instant)>> {
    QR_CONFIRMED_TOKENS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 开始扫码：拉取登录二维码（登录页链接或内联 PNG）与轮询用的
/// `qrcode_id`。端点免鉴权，凭据在扫码确认后才产生。
pub async fn post_wechat_qr_start() -> Result<HttpResponse, AppError> {
    let qr = crate::connect::platforms::wechat::fetch_login_qrcode(
        crate::connect::platforms::wechat::ilink_default_base_url(),
    )
    .await
    .map_err(AppError::BadRequest)?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "qrcode_id": qr.qrcode_id,
        "image_kind": qr.image_kind,
        "image": qr.image_content,
        "url": qr.url,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WechatQrStatusRequest {
    qrcode_id: String,
}

/// 查询扫码状态一次（前端 2–3 秒轮询）。`confirmed` 时新 token 进入
/// 服务器侧短 TTL 缓存，响应只报状态不回 token。
pub async fn post_wechat_qr_status(
    payload: web::Json<WechatQrStatusRequest>,
) -> Result<HttpResponse, AppError> {
    let qrcode_id = payload.into_inner().qrcode_id;
    let status = crate::connect::platforms::wechat::poll_login_qrcode(
        crate::connect::platforms::wechat::ilink_default_base_url(),
        &qrcode_id,
    )
    .await
    .map_err(AppError::BadRequest)?;
    if status.status == "confirmed" {
        if let Some(token) = status.token {
            let mut cache = qr_token_cache().lock().expect("wechat qr token cache lock");
            cache.insert(qrcode_id, (token, std::time::Instant::now()));
            // 扫码后未 apply 的确认项随插入清扫，缓存不会无限增长。
            cache.retain(|_, (_, cached_at)| cached_at.elapsed() <= QR_TOKEN_TTL);
        }
    }
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "status": status.status,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WechatQrApplyRequest {
    qrcode_id: String,
    expected_revision: u64,
    /// 已存在微信平台条目时传入其稳定 id（凭据归属不漂移）。
    id: Option<String>,
    #[serde(default)]
    allow_from: Vec<String>,
    #[serde(default)]
    admin_from: Vec<String>,
    voice: Option<bamboo_config::WechatVoiceConfig>,
    voice_api_key_change: Option<CredentialAction>,
}

/// 扫码确认后落库：用缓存中的新 token 构造与手填完全一致的 connect
/// 变更（token 走凭据管道替换，凭据永不回浏览器）。
pub async fn post_wechat_qr_apply(
    app_state: web::Data<AppState>,
    payload: web::Json<WechatQrApplyRequest>,
) -> Result<HttpResponse, AppError> {
    let request = payload.into_inner();
    let cached = {
        let mut cache = qr_token_cache().lock().expect("wechat qr token cache lock");
        cache.remove(&request.qrcode_id)
    };
    let Some((token, cached_at)) = cached else {
        return Err(AppError::BadRequest(
            "扫码结果已过期或未确认，请重新扫码".to_string(),
        ));
    };
    if cached_at.elapsed() > QR_TOKEN_TTL {
        return Err(AppError::BadRequest(
            "扫码结果已过期（超过 5 分钟），请重新扫码".to_string(),
        ));
    }
    let allow_from = request.allow_from;
    // 扫码 = 操作者证明：未显式提供 allow_from 时默认开启"信任首个发信人"
    // 闩锁——扫码接口不返回扫码人 ID，首条入站消息的发送者自动获准并
    // 由平台持久化（v0.0.4）。
    let trust_first_sender = allow_from.is_empty();
    let mutation = ConnectMutationRequest {
        expected_revision: request.expected_revision,
        data: ConnectMutationData {
            platforms: vec![ConnectPlatformMutation {
                id: request.id,
                project_id: None,
                platform_type: "wechat".to_string(),
                app_id: None,
                domain: None,
                allow_from,
                admin_from: request.admin_from,
                voice: request.voice,
                trust_first_sender: Some(trust_first_sender),
                typing_indicator: None,
                dream: None,
                auto_project: None,
                token_change: Some(CredentialAction::Replace { value: token }),
                app_secret_change: None,
                voice_api_key_change: request.voice_api_key_change,
            }],
        },
    };
    apply_connect_mutation(&app_state, mutation).await
}

// ---------------------------------------------------------------------------
// connect 会话管理（设置页微信连接区块的会话面板）：列表 / 新建 / 切换 /
// 删除。全部作用在 ConnectManager 持有的实时桥接上（会话映射、忙锁、
// 挂起审批），与聊天内的 /sessions、/switch、/del、/new 命令同源。
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectSessionsQuery {
    platform: Option<String>,
}

/// `GET /api/v1/bamboo/connect/sessions?platform=wechat`
pub async fn get_connect_sessions(
    app_state: web::Data<AppState>,
    query: web::Query<ConnectSessionsQuery>,
) -> Result<HttpResponse, AppError> {
    let platform = query
        .platform
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(platform) = platform {
        if !matches!(platform, "telegram" | "feishu" | "wechat") {
            return Err(AppError::BadRequest(format!(
                "unsupported connect platform '{platform}'"
            )));
        }
    }
    let sessions = app_state
        .connect_manager
        .bridge()
        .list_connect_sessions(platform)
        .await;
    Ok(HttpResponse::Ok().json(serde_json::json!({ "sessions": sessions })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateConnectSessionRequest {
    platform: String,
    chat_id: String,
    /// 缺省与 chat_id 相同（微信私聊两者一致）。
    #[serde(default)]
    user_id: Option<String>,
}

/// `POST /api/v1/bamboo/connect/sessions` — 为指定聊天立即新建会话
/// （聊天内 `/new` 的"马上创建"版本）。
pub async fn post_connect_session(
    app_state: web::Data<AppState>,
    payload: web::Json<CreateConnectSessionRequest>,
) -> Result<HttpResponse, AppError> {
    let request = payload.into_inner();
    let platform = request.platform.trim().to_string();
    if !matches!(platform.as_str(), "telegram" | "feishu" | "wechat") {
        return Err(AppError::BadRequest(format!(
            "unsupported connect platform '{platform}'"
        )));
    }
    if request.chat_id.trim().is_empty() {
        return Err(AppError::BadRequest("chat_id must be nonempty".to_string()));
    }
    let user_id = request
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| request.chat_id.trim().to_string());
    let key = crate::connect::SessionKey {
        platform,
        chat_id: request.chat_id.trim().to_string(),
        user_id,
    }
    .as_string();
    let bridge = app_state.connect_manager.bridge();
    let session = bridge
        .create_session_for_key(&key)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "session_id": session.id,
        "connect_key": key,
        "title": session.title,
        "project_id": session.metadata.get("project_id"),
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivateConnectSessionRequest {
    connect_key: String,
}

/// `POST /api/v1/bamboo/connect/sessions/{session_id}/activate` — 把会话
/// 所属聊天的当前会话切回这个历史会话（聊天内 `/switch` 同款）。
pub async fn post_connect_session_activate(
    app_state: web::Data<AppState>,
    path: web::Path<String>,
    payload: web::Json<ActivateConnectSessionRequest>,
) -> Result<HttpResponse, AppError> {
    let session_id = path.into_inner();
    let key = payload.into_inner().connect_key;
    app_state
        .connect_manager
        .bridge()
        .activate_session_for_key(key.trim(), &session_id)
        .await
        .map_err(AppError::BadRequest)?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "session_id": session_id,
    })))
}

/// `DELETE /api/v1/bamboo/connect/sessions/{session_id}` — 先摘除桥接映射
/// 与挂起状态，再复用通用会话删除处理器（技能激活释放、运行取消、存储
/// 删除、账户事件广播等全量逻辑）。
pub async fn delete_connect_session(
    app_state: web::Data<AppState>,
    path: web::Path<String>,
) -> Result<HttpResponse, AppError> {
    let session_id = path.into_inner();
    app_state
        .connect_manager
        .bridge()
        .detach_connect_session(&session_id)
        .await;
    crate::handlers::agent::delete::handler(app_state.clone(), web::Path::from(session_id))
        .await
        .map_err(|error| {
            AppError::InternalError(anyhow::anyhow!("failed to delete connect session: {error}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{http::StatusCode, test, App};

    #[actix_web::test]
    async fn action_debug_never_exposes_connect_secrets() {
        let payload: ConnectMutationRequest = serde_json::from_value(serde_json::json!({
            "expected_revision": 0,
            "data": {
                "platforms": [{
                    "type": "telegram",
                    "token_change": {"action": "replace", "value": "telegram-secret"}
                }]
            }
        }))
        .unwrap();
        let debug = format!("{payload:?}");
        assert!(!debug.contains("telegram-secret"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[actix_web::test]
    async fn dedicated_connect_mutation_uses_section_cas_and_explicit_actions() {
        let _key = bamboo_config::encryption::set_test_encryption_key([0xc5; 32]);
        let dir = tempfile::tempdir().unwrap();
        let state = web::Data::new(AppState::new(dir.path().to_path_buf()).await.unwrap());
        let mut events = state.account_sink.subscribe();
        let app = test::init_service(
            App::new()
                .app_data(state.clone())
                .route("/connect", web::put().to(put_connect_config)),
        )
        .await;

        let replace = test::call_service(
            &app,
            test::TestRequest::put()
                .uri("/connect")
                .set_json(serde_json::json!({
                    "expected_revision": 0,
                    "data": {
                        "platforms": [{
                            "type": "telegram",
                            "allow_from": ["owner"],
                            "token_change": {
                                "action": "replace",
                                "value": "telegram-token-secret"
                            }
                        }]
                    }
                }))
                .to_request(),
        )
        .await;
        assert_eq!(replace.status(), StatusCode::OK);
        let replace: serde_json::Value = test::read_body_json(replace).await;
        assert_eq!(replace["revision"], 1);
        assert_eq!(replace["section"]["revision"], 1);
        assert_eq!(replace["credentials"][0]["configured"], true);
        assert_eq!(replace["credentials"][0]["state"], "configured");
        assert!(!replace.to_string().contains("telegram-token-secret"));
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let event = events.recv().await.unwrap();
                if matches!(
                    &event.event,
                    bamboo_agent_core::AgentEvent::ConfigChanged { section, revision: 1 }
                        if section == "connect"
                ) {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            event.event,
            bamboo_agent_core::AgentEvent::ConfigChanged { ref section, revision: 1 }
                if section == "connect"
        ));
        let platform_id = replace["section"]["data"]["platforms"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let stale = test::call_service(
            &app,
            test::TestRequest::put()
                .uri("/connect")
                .set_json(serde_json::json!({
                    "expected_revision": 0,
                    "data": {
                        "platforms": [{
                            "id": platform_id,
                            "type": "telegram",
                            "allow_from": ["stale"],
                            "token_change": {
                                "action": "replace",
                                "value": "stale-telegram-secret"
                            }
                        }]
                    }
                }))
                .to_request(),
        )
        .await;
        assert_eq!(stale.status(), StatusCode::CONFLICT);
        assert!(!String::from_utf8(test::read_body(stale).await.to_vec())
            .unwrap()
            .contains("stale-telegram-secret"));

        let keep = test::call_service(
            &app,
            test::TestRequest::put()
                .uri("/connect")
                .set_json(serde_json::json!({
                    "expected_revision": 1,
                    "data": {
                        "platforms": [{
                            "id": platform_id,
                            "type": "telegram",
                            "allow_from": ["owner", "maintainer"],
                            "token_change": {"action": "keep"}
                        }]
                    }
                }))
                .to_request(),
        )
        .await;
        assert_eq!(keep.status(), StatusCode::OK);
        let keep: serde_json::Value = test::read_body_json(keep).await;
        assert_eq!(keep["section"]["revision"], 2);
        assert_eq!(
            state.config.read().await.connect.platforms[0]
                .token
                .as_deref(),
            Some("telegram-token-secret")
        );

        let clear = test::call_service(
            &app,
            test::TestRequest::put()
                .uri("/connect")
                .set_json(serde_json::json!({
                    "expected_revision": 2,
                    "data": {
                        "platforms": [{
                            "id": platform_id,
                            "type": "telegram",
                            "allow_from": ["owner", "maintainer"],
                            "token_change": {"action": "clear"}
                        }]
                    }
                }))
                .to_request(),
        )
        .await;
        assert_eq!(clear.status(), StatusCode::OK);
        let clear: serde_json::Value = test::read_body_json(clear).await;
        assert_eq!(clear["section"]["revision"], 3);
        assert!(clear["credentials"].as_array().unwrap().is_empty());
        assert_eq!(
            clear["credential_status"][&platform_id]["token"]["state"],
            "missing"
        );
        assert!(state.config.read().await.connect.platforms[0]
            .token
            .is_none());

        let masked = test::call_service(
            &app,
            test::TestRequest::put()
                .uri("/connect")
                .set_json(serde_json::json!({
                    "expected_revision": 3,
                    "data": {
                        "platforms": [{
                            "id": platform_id,
                            "type": "telegram",
                            "token_change": {
                                "action": "replace",
                                "value": "********"
                            }
                        }]
                    }
                }))
                .to_request(),
        )
        .await;
        assert_eq!(masked.status(), StatusCode::BAD_REQUEST);

        let connect_file = std::fs::read_to_string(dir.path().join("connect.json")).unwrap();
        let credential_file = std::fs::read_to_string(dir.path().join("credentials.json")).unwrap();
        for secret in ["telegram-token-secret", "stale-telegram-secret", "********"] {
            assert!(!connect_file.contains(secret));
            assert!(!credential_file.contains(secret));
        }
    }

    /// voice 段（微信语音能力）经专用 PUT 端点写入、原样持久化；**缺省
    /// 提交（不含 voice 字段）必须保留现有 voice**——不认识该字段的旧
    /// 客户端整单保存不能把它抹掉（回归：曾硬编码为 None 导致配置被清）。
    #[actix_web::test]
    async fn dedicated_connect_mutation_supports_and_preserves_voice() {
        let _key = bamboo_config::encryption::set_test_encryption_key([0xc5; 32]);
        let dir = tempfile::tempdir().unwrap();
        let state = web::Data::new(AppState::new(dir.path().to_path_buf()).await.unwrap());
        let app = test::init_service(
            App::new()
                .app_data(state.clone())
                .route("/connect", web::put().to(put_connect_config)),
        )
        .await;

        // 1) 带 voice 写入。
        let create = test::call_service(
            &app,
            test::TestRequest::put()
                .uri("/connect")
                .set_json(serde_json::json!({
                    "expected_revision": 0,
                    "data": {
                        "platforms": [{
                            "type": "wechat",
                            "allow_from": ["wxid_owner@im.wechat"],
                            "token_change": {"action": "replace", "value": "wechat-token"},
                            "voice_api_key_change": {"action": "replace", "value": "sk-voice-1"},
                            "voice": {
                                "reply_mode": "mirror",
                                "file_asr": "on"
                            }
                        }]
                    }
                }))
                .to_request(),
        )
        .await;
        let diag_status = create.status();
        let diag_body = String::from_utf8(test::read_body(create).await.to_vec()).unwrap();
        eprintln!("DIAG create status={diag_status:?} body={diag_body}");
        assert!(diag_status.is_success(), "create failed: {diag_body}");
        let create: serde_json::Value = serde_json::from_str(&diag_body).unwrap();
        let platform_id = create["section"]["data"]["platforms"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            create["section"]["data"]["platforms"][0]["voice"]["reply_mode"],
            "mirror"
        );
        // 密钥绝不明文落盘（加密管道）；voice 非秘密参数正常持久化。
        let connect_file = std::fs::read_to_string(dir.path().join("connect.json")).unwrap();
        assert!(
            !connect_file.contains("sk-voice-1"),
            "plaintext key must never hit disk"
        );
        assert!(connect_file.contains("reply_mode")); // pretty-print 形态不定，只验字段存在
        assert!(state.config.read().await.connect.platforms[0].voice_api_key_configured);
        assert_eq!(
            state.config.read().await.connect.platforms[0]
                .voice_api_key
                .as_deref(),
            Some("sk-voice-1")
        );
        assert!(state.config.read().await.connect.platforms[0]
            .voice
            .as_ref()
            .is_some_and(|voice| voice.reply_mode.as_deref() == Some("mirror")));

        // 2) 不含 voice 的整单保存（旧客户端形态）——voice 必须保留。
        let keep = test::call_service(
            &app,
            test::TestRequest::put()
                .uri("/connect")
                .set_json(serde_json::json!({
                    "expected_revision": 1,
                    "data": {
                        "platforms": [{
                            "id": platform_id,
                            "type": "wechat",
                            "allow_from": ["wxid_owner@im.wechat", "wxid_other@im.wechat"],
                            "token_change": {"action": "keep"}
                        }]
                    }
                }))
                .to_request(),
        )
        .await;
        assert_eq!(keep.status(), StatusCode::OK);
        assert_eq!(
            state.config.read().await.connect.platforms[0]
                .voice_api_key
                .as_deref(),
            Some("sk-voice-1"),
            "absent voice/key must preserve the existing config"
        );

        // 3) 显式替换 voice（提交即整体覆盖）。
        let replace = test::call_service(
            &app,
            test::TestRequest::put()
                .uri("/connect")
                .set_json(serde_json::json!({
                    "expected_revision": 2,
                    "data": {
                        "platforms": [{
                            "id": platform_id,
                            "type": "wechat",
                            "allow_from": ["wxid_owner@im.wechat"],
                            "token_change": {"action": "keep"},
                            "voice_api_key_change": {"action": "replace", "value": "sk-voice-2"},
                            "voice": {
                                "reply_mode": "always"
                            }
                        }]
                    }
                }))
                .to_request(),
        )
        .await;
        assert_eq!(replace.status(), StatusCode::OK);
        assert_eq!(
            state.config.read().await.connect.platforms[0]
                .voice_api_key
                .as_deref(),
            Some("sk-voice-2")
        );
        let voice = state.config.read().await.connect.platforms[0]
            .voice
            .clone()
            .expect("voice present");
        assert_eq!(voice.reply_mode.as_deref(), Some("always"));
        // 未提交的 file_asr 回到默认（整体替换，不做字段级合并）。
        assert_eq!(voice.file_asr, None);

        // 4) voice 密钥不出现在 Debug 输出之外的正则担忧不适用（非秘密），
        //    但凭据文件不应混入 voice 密钥。
        let credential_file = std::fs::read_to_string(dir.path().join("credentials.json")).unwrap();
        assert!(!credential_file.contains("sk-voice"));
    }
}
