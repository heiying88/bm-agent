//! NeedsHuman/QuestionDialog → buttons/text replies → respond path (epic
//! #447's planned `approvals.rs`, issue #458).
//!
//! Three concerns live here:
//! - Rendering a [`PendingAsk`][render_pending_ask_type] as an outbound
//!   message (buttons when the platform supports them, always ALSO a
//!   numbered text list — text replies are first-class on every platform).
//! - Matching an inbound text reply or button `callback_data` against a
//!   [`ParkedAsk`], including the binary-ask keyword mapping
//!   (允许/yes/allow vs deny/no).
//! - The [`Responder`] seam: the ONLY resolution path is
//!   `bamboo_engine::session_app::respond::submit_pending_response` followed
//!   by `resume::resume_session_execution` — exactly what
//!   `POST /sessions/{id}/respond` does. [`EngineResponder`] is the
//!   production implementation (in-proc, via [`super::bridge::ConnectContext`]);
//!   tests inject a fake instead of standing up a full `AppState`.
//!
//! [render_pending_ask_type]: crate::connect::render::PendingAsk

use std::sync::Arc;

use tokio::sync::broadcast;

use bamboo_agent_core::tools::ToolExecutionContext;
use bamboo_agent_core::{AgentEvent, Session};
use bamboo_engine::execution::{
    create_event_forwarder_with_history_commit_barrier, get_or_create_event_sender,
    reserve_session_execution, SessionExecutionReserveOutcome,
};
use bamboo_engine::runtime::execution::agent_spawn::{
    spawn_session_execution, SessionExecutionArgs,
};
use bamboo_engine::session_app::approval_replay::{
    apply_permission_replay_result, find_permission_replay_target, refresh_approval_replay_posture,
    repark_permission_replay, restore_permission_replay_authorization,
    validate_permission_replay_authority, ApprovalReplayDecision, PermissionReplayTarget,
};
use bamboo_engine::session_app::execute::consume_pending_clarification_resume;
use bamboo_engine::session_app::resolution::resolve_resume_config_snapshot;
use bamboo_engine::session_app::respond::{
    acquire_pending_response_guard, inspect_pending_response_guarded,
    submit_pending_permission_response_checked_guarded, submit_pending_response_checked_guarded,
    validate_pending_response, PERMISSION_REEXECUTE_GENERATION_METADATA_KEY,
    PERMISSION_REEXECUTE_METADATA_KEY,
};
use bamboo_engine::session_app::resume::{ResumeExecutionPort, ResumeSpawnRequest};
use bamboo_engine::session_app::types::RespondInput;
use bamboo_engine::{ModelRoster, RoleModel};
use bamboo_tools::permission::{
    PermissionDecision, PermissionDecisionKind, PermissionDecisionReceipt, PermissionMatcher,
    PermissionMatcherKind,
};

use super::bridge::ConnectContext;
use super::platform::{Button, OutboundMessage, Platform, PlatformResult, ReplyCtx};
use super::render::PendingAsk;

/// Longest a button's visible label is allowed to be — Telegram (and most IM
/// platforms) truncate/reject very long inline-button text, so keep it well
/// under any known limit.
const BUTTON_LABEL_MAX_CHARS: usize = 48;

// ---------------------------------------------------------------------------
// ParkedAsk — the bridge's one-ask-per-chat state
// ---------------------------------------------------------------------------

/// A pending question rendered to a chat and awaiting resolution (button
/// press or text reply). One per chat at a time (issue #458: "one parked ask
/// per chat — session serializes asks").
#[derive(Debug, Clone)]
pub struct ParkedAsk {
    /// Short nonce embedded in every button's `callback_data`
    /// (`"{nonce}:{option_index}"`). Validated on every callback so
    /// forged/stale data is ignored.
    pub nonce: String,
    pub session_id: String,
    /// Exact durable identity used as the response CAS guard. Legacy live
    /// events are reconciled against session persistence before a `ParkedAsk`
    /// can be constructed, so a new Connect client never submits an
    /// unguarded clarification answer.
    pub tool_call_id: String,
    pub tool_name: String,
    pub question: String,
    pub options: Vec<String>,
    pub allow_custom: bool,
}

impl ParkedAsk {
    pub fn new(nonce: String, session_id: String, ask: &PendingAsk) -> Option<Self> {
        Some(Self {
            nonce,
            session_id,
            tool_call_id: ask.tool_call_id.clone()?,
            tool_name: ask.tool_name.clone(),
            question: ask.question.clone(),
            options: ask.options.clone(),
            allow_custom: ask.allow_custom,
        })
    }
}

/// A short, hard-to-guess nonce for one parked ask. Not cryptographically
/// load-bearing on its own (it's paired with per-chat scoping + a single
/// live ask at a time) — just enough entropy that a stale/forged
/// `callback_data` from a different ask/session won't collide by accident.
pub fn new_nonce() -> String {
    let raw = uuid::Uuid::new_v4().to_string();
    raw.split('-').next().unwrap_or(&raw).to_string()
}

// ---------------------------------------------------------------------------
// Rendering an ask
// ---------------------------------------------------------------------------

fn truncate_label(text: &str) -> String {
    if text.chars().count() <= BUTTON_LABEL_MAX_CHARS {
        return text.to_string();
    }
    let mut out: String = text.chars().take(BUTTON_LABEL_MAX_CHARS - 1).collect();
    out.push('…');
    out
}

/// 权限提示的选项固定为 Approve/Deny（engine 的 permission gate 只合成
/// 这两个选项）。
fn is_permission_ask(options: &[String]) -> bool {
    options.len() == 2
        && options[0].eq_ignore_ascii_case("approve")
        && options[1].eq_ignore_ascii_case("deny")
}

/// 权限提示展示层统一换成中文标签，更贴合 IM 用户；存储的英文
/// 选项仍是应答匹配词表——数字（"1"/"2"）、英文原文（"approve"）、
/// 中英文肯定/否定关键词（"允许"/"拒绝"）都能命中，见 `match_text_answer`。
fn option_display_label(options: &[String], index: usize) -> &str {
    if is_permission_ask(options) {
        if index == 0 {
            "允许"
        } else {
            "拒绝"
        }
    } else {
        options[index].as_str()
    }
}

/// 权限提示正文整体汉化（仅展示层；应答匹配仍基于存储的英文选项）。
/// engine 的 permission gate 生成固定英文模板：
/// `**Permission required**\n\nThe \`{tool}\` tool needs approval to {action} on:\n\n\`{resource}\``
/// 解析失败或未知动作时原样返回，绝不阻断渲染。
fn translate_permission_question(question: &str) -> String {
    let trimmed = question.trim();
    let Some(body) = trimmed
        .strip_prefix("**Permission required**")
        .map(str::trim)
        .and_then(|rest| rest.strip_prefix("The `"))
    else {
        return trimmed.to_string();
    };
    let Some((tool, tail)) = body.split_once("` tool needs approval to ") else {
        return trimmed.to_string();
    };
    let Some((action, resource_part)) = tail.split_once(" on:") else {
        return trimmed.to_string();
    };
    let resource = resource_part.trim().trim_matches('`');
    let action_cn = match action.trim() {
        "Write files to disk" => "写入文件",
        "Execute shell commands" => "执行命令",
        "Perform Git write operations (commit, push, etc.)" => "执行 Git 写操作（commit、push 等）",
        "Make HTTP requests to external services" => "发起外部 HTTP 请求",
        "Delete files or directories" => "删除文件或目录",
        "Run interactive terminal sessions" => "运行交互式终端会话",
        "Interact with a browser page" => "操作浏览器页面",
        _ => {
            return format!("⚠️ 需要权限确认\n\n{action}：\n\n{resource}");
        }
    };
    format!("⚠️ 需要权限确认：{action_cn}\n\n{resource}\n\n（来自工具 {tool}）")
}

/// Format the ask's question + a numbered option list (text replies remain
/// first-class even when buttons are ALSO rendered).
fn format_ask_text(ask: &ParkedAsk) -> String {
    let question = if is_permission_ask(&ask.options) {
        translate_permission_question(&ask.question)
    } else {
        ask.question.clone()
    };
    let mut text = question;
    if !ask.options.is_empty() {
        text.push_str("\n\n");
        for (index, _option) in ask.options.iter().enumerate() {
            text.push_str(&format!(
                "{}. {}\n",
                index + 1,
                option_display_label(&ask.options, index)
            ));
        }
        if is_permission_ask(&ask.options) {
            text.push_str("（回复数字选择；回复\"都允许\"本次会话内不再逐条询问）");
        } else {
            text.push_str("（回复数字即可选择）");
        }
    }
    if ask.allow_custom {
        text.push_str("\n(or reply with your own answer)");
    }
    text
}

/// Render `ask` to the chat: inline buttons (one per option, `callback_data =
/// "{nonce}:{index}"`) when `buttons_capable`, always alongside the numbered
/// text list — per issue #458, buttons are an enhancement, never a
/// requirement. Returns the platform error (if the send failed) so the
/// caller can log it; rendering failure does not itself invalidate the
/// parked ask (a text reply can still resolve it).
pub async fn render_ask(
    platform: &Arc<dyn Platform>,
    reply_ctx: &ReplyCtx,
    ask: &ParkedAsk,
    buttons_capable: bool,
) -> PlatformResult<()> {
    let text = format_ask_text(ask);
    let outbound = if buttons_capable && !ask.options.is_empty() {
        let rows: Vec<Vec<Button>> = ask
            .options
            .iter()
            .enumerate()
            .map(|(index, _option)| {
                vec![Button::new(
                    truncate_label(option_display_label(&ask.options, index)),
                    format!("{}:{index}", ask.nonce),
                )]
            })
            .collect();
        OutboundMessage::text(text).with_buttons(rows)
    } else {
        OutboundMessage::text(text)
    };
    platform.reply(reply_ctx, outbound).await.map(|_| ())
}

/// Render a legacy/external clarification that cannot be matched to a durable
/// pending tool call. It remains fully inspectable, but deliberately has no
/// buttons or reply instructions and is never registered as answerable chat
/// state.
pub async fn render_read_only_ask(
    platform: &Arc<dyn Platform>,
    reply_ctx: &ReplyCtx,
    ask: &PendingAsk,
    reason: &str,
) -> PlatformResult<()> {
    let mut text = if is_permission_ask(&ask.options) {
        translate_permission_question(&ask.question)
    } else {
        ask.question.clone()
    };
    if !ask.options.is_empty() {
        text.push_str("\n\n");
        for (index, option) in ask.options.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", index + 1, option));
        }
    }
    text.push_str("\n(Response unavailable: ");
    text.push_str(reason);
    text.push(')');
    platform
        .reply(reply_ctx, OutboundMessage::text(text))
        .await
        .map(|_| ())
}

// ---------------------------------------------------------------------------
// Matching a text reply / callback against a ParkedAsk
// ---------------------------------------------------------------------------

const AFFIRMATIVE_KEYWORDS: &[&str] = &[
    "允许", "同意", "确定", "是", "yes", "allow", "approve", "ok",
];
/// "stay" comes from plan-mode's decline phrasing — ExitPlanMode's negative
/// option is literally "Stay in plan mode" (see
/// `session_app::respond::is_exit_plan_mode_approved`), so a user typing
/// "stay" declines the plan approval. Safe to keep in this fallback list
/// because [`match_text_answer`] tries EXACT (case-insensitive) option-text
/// matching BEFORE the keyword fallback: an ask whose positive option is
/// literally titled "Stay" resolves on the exact match and never reaches
/// here.
const NEGATIVE_KEYWORDS: &[&str] = &["拒绝", "不", "否", "no", "deny", "reject", "stay"];
/// 跳过/取消意图（跨语言）：回复"跳过"可命中英文选项 "Skip"，反之亦然。
const SKIP_KEYWORDS: &[&str] = &["跳过", "略过", "忽略", "skip", "ignore"];
const CANCEL_KEYWORDS: &[&str] = &["取消", "算了", "cancel", "abort"];

/// 在选项里找包含任一关键词的选项（小写比较）。
fn pick_option_by_keyword(options: &[String], keywords: &[&str]) -> Option<String> {
    options
        .iter()
        .find(|option| {
            let lower = option.to_lowercase();
            keywords.iter().any(|keyword| lower.contains(keyword))
        })
        .cloned()
}

fn classify_intent(text: &str) -> Option<bool> {
    let lower = text.trim().to_lowercase();
    if AFFIRMATIVE_KEYWORDS.iter().any(|keyword| lower == *keyword) {
        return Some(true);
    }
    if NEGATIVE_KEYWORDS.iter().any(|keyword| lower == *keyword) {
        return Some(false);
    }
    None
}

/// "First-affirmative mapping": prefer an option whose OWN text already
/// reads as affirmative/negative (e.g. "Approve" / "Deny"); for a plain
/// 2-option ask with no such wording, fall back to treating the first option
/// as the affirmative one.
fn pick_option_by_intent(options: &[String], affirmative: bool) -> Option<String> {
    let keywords: &[&str] = if affirmative {
        AFFIRMATIVE_KEYWORDS
    } else {
        NEGATIVE_KEYWORDS
    };
    if let Some(option) = options.iter().find(|option| {
        let lower = option.to_lowercase();
        keywords.iter().any(|keyword| lower.contains(keyword))
    }) {
        return Some(option.clone());
    }
    if options.len() == 2 {
        return Some(if affirmative {
            options[0].clone()
        } else {
            options[1].clone()
        });
    }
    None
}

/// 判断聊天回复是否表达"接下来都允许"这类**会话级持续授权**意图。
/// 命中后本次仍按"允许"应答（AllowOnce 回执），并额外安装一个会话
/// 范围的类型化授权——本会话后续同类操作不再逐条询问。负向词优先：
/// "以后都不允许"不会被判成持续授权。
pub fn wants_session_scope(text: &str) -> bool {
    const SESSION_ALLOW_PHRASES: &[&str] = &[
        "都允许",
        "全部允许",
        "总是允许",
        "一律允许",
        "始终允许",
        "都同意",
        "全部同意",
        "不再询问",
        "别再问",
        "不用再问",
        "别问了",
        "always allow",
        "allow all",
        "stop asking",
        "don't ask again",
    ];
    const NEGATIVE_MARKERS: &[&str] =
        &["不允许", "不准", "不要", "拒绝", "不同意", "deny", "reject"];
    let trimmed = text.trim();
    let lower = trimmed.to_lowercase();
    trimmed.chars().count() <= 40
        && SESSION_ALLOW_PHRASES
            .iter()
            .any(|phrase| lower.contains(phrase))
        && !NEGATIVE_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Match a text reply against `ask`, returning the answer to submit, or
/// `None` when it doesn't resolve the ask at all (issue #458: a non-matching
/// text on a CLOSED ask — no free text allowed — falls through to the
/// caller's normal busy-queue handling instead of being submitted as a
/// doomed-to-fail answer).
///
/// Tried in order: 1-based numeric option index, exact (case-insensitive)
/// option text, session-scope allow phrases ("接下来都允许"), then — for a
/// closed (non-`allow_custom`) ask — the affirmative/negative keyword
/// mapping. An OPEN ask (`allow_custom`) always
/// matches: any non-empty text IS the answer, verbatim (matching
/// `validate_pending_response`'s server-side rule).
pub fn match_text_answer(ask: &ParkedAsk, text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(index) = trimmed.parse::<usize>() {
        if index >= 1 && index <= ask.options.len() {
            return Some(ask.options[index - 1].clone());
        }
    }
    if let Some(option) = ask
        .options
        .iter()
        .find(|option| option.eq_ignore_ascii_case(trimmed))
    {
        return Some(option.clone());
    }
    if !ask.allow_custom && wants_session_scope(trimmed) {
        // "接下来都允许"这类持续授权短语：本次映射到肯定选项，会话级
        // 授权由 `typed_permission_receipt` 的 session_scope 参数安装。
        if let Some(option) = pick_option_by_intent(&ask.options, true) {
            return Some(option.clone());
        }
    }
    if !ask.allow_custom {
        if let Some(intent) = classify_intent(trimmed) {
            if let Some(option) = pick_option_by_intent(&ask.options, intent) {
                return Some(option.clone());
            }
        }
    }
    if !ask.allow_custom {
        // 通用子串层：短回复唯一命中一个选项（"跳过"→"跳过该文件"、
        // "skip"→"Skip"）。仅当恰好一个选项包含该词时生效，避免歧义。
        let lower = trimmed.to_lowercase();
        if (2..=12).contains(&lower.chars().count()) {
            let hits: Vec<&String> = ask
                .options
                .iter()
                .filter(|option| option.to_lowercase().contains(&lower))
                .collect();
            if hits.len() == 1 {
                return Some(hits[0].clone());
            }
        }
        // 跨语言意图层：回复"跳过"命中英文选项 "Skip"，回复"取消"命中
        // "Cancel"（选项文字与用户语言不一致时兜底）。
        for keywords in [SKIP_KEYWORDS, CANCEL_KEYWORDS] {
            if keywords.iter().any(|keyword| lower == *keyword) {
                if let Some(option) = pick_option_by_keyword(&ask.options, keywords) {
                    return Some(option);
                }
            }
        }
    }
    if ask.allow_custom {
        return Some(trimmed.to_string());
    }
    None
}

/// Match a button press's `callback_data` (`"{nonce}:{index}"`) against
/// `ask`. Returns `None` for anything that doesn't EXACTLY match the parked
/// nonce and a valid option index — forged/stale data (issue #458: "always
/// answerCallbackQuery, even stale" — the caller acks regardless, but never
/// forwards a non-match as an answer).
pub fn match_callback_data(ask: &ParkedAsk, data: &str) -> Option<String> {
    let (nonce, index_str) = data.split_once(':')?;
    if nonce != ask.nonce {
        return None;
    }
    let index: usize = index_str.parse().ok()?;
    ask.options.get(index).cloned()
}

// ---------------------------------------------------------------------------
// Responder — the resolution seam
// ---------------------------------------------------------------------------

/// What happened after submitting an answer and attempting to resume.
pub enum RespondAndResumeOutcome {
    /// Execution resumed; `receiver` was subscribed BEFORE the resume was
    /// triggered, so the caller can keep rendering without missing events.
    Resumed(broadcast::Receiver<AgentEvent>),
    /// The answer was recorded, but nothing (more) is running — e.g. the
    /// runner slot was already taken by a concurrent run, or the session
    /// vanished between answering and resuming. `reason` is a short,
    /// user-facing explanation.
    NotResumed(String),
}

/// Error submitting an answer (mirrors `bamboo_engine::session_app::errors::RespondError`,
/// decoupled so `connect` doesn't leak that error type through its public
/// surface).
#[derive(Debug, thiserror::Error)]
pub enum ResponderError {
    #[error("session not found")]
    NotFound,
    #[error("no pending question waiting for a response")]
    NoPendingQuestion,
    #[error("the pending question changed; this action has expired")]
    PendingQuestionChanged,
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("{0}")]
    Other(String),
}

/// The bridge's resolution seam (issue #458: "Design a small Responder seam
/// on the bridge so tests inject a fake instead of full AppState"). The ONLY
/// production implementation ([`EngineResponder`]) routes through
/// `submit_pending_response` + `resume_session_execution` — the exact same
/// use-case functions `POST /sessions/{id}/respond` calls — never a parallel
/// path.
#[async_trait::async_trait]
pub trait Responder: Send + Sync {
    async fn respond_and_resume(
        &self,
        session_id: &str,
        expected_tool_call_id: Option<&str>,
        answer: String,
        session_scope: bool,
    ) -> Result<RespondAndResumeOutcome, ResponderError>;
}

fn map_respond_error(error: bamboo_engine::session_app::errors::RespondError) -> ResponderError {
    use bamboo_engine::session_app::errors::RespondError;
    match error {
        RespondError::NotFound(_) => ResponderError::NotFound,
        RespondError::NoPendingQuestion => ResponderError::NoPendingQuestion,
        RespondError::PendingQuestionMismatch { .. } => ResponderError::PendingQuestionChanged,
        RespondError::InvalidResponse(message) => ResponderError::InvalidResponse(message),
        other => ResponderError::Other(other.to_string()),
    }
}

/// Production [`Responder`]: submits through `submit_pending_response`
/// (`SessionAccess` is implemented directly by `SessionRepository`, no
/// `AppState` wrapper needed), applies any permission grants the answer
/// implied (mirrors `handlers::agent::respond::handlers::submit`), then
/// resumes via [`ConnectResumePort`] — the connect-scoped
/// `ResumeExecutionPort` implementation that spawns through the same
/// crate-agnostic `spawn_session_execution` the bridge already uses for a
/// fresh prompt (`bridge::ConnectBridge::run_prompt`), including re-running a
/// gated tool call that was only a placeholder while awaiting approval.
pub struct EngineResponder {
    ctx: ConnectContext,
}

impl EngineResponder {
    pub fn new(ctx: ConnectContext) -> Self {
        Self { ctx }
    }
}

/// 安全拒绝恢复时必须给等待中的渲染流一个终结事件：只写日志直接
/// return 的话，事件通道不会再有任何输出，桥接的 stream_execution
/// 永远收不到事件、会话 busy 标记无法释放——用户侧就是"选了没反应、
/// 卡死到 /new"。`AgentEvent::Error` 会被渲染为 "Error: …" 并正常结束
/// 本轮，会话保持可继续对话。
async fn refuse_resume(mpsc_tx: &tokio::sync::mpsc::Sender<AgentEvent>, message: String) {
    let _ = mpsc_tx.send(AgentEvent::Error { message }).await;
}

#[async_trait::async_trait]
impl Responder for EngineResponder {
    async fn respond_and_resume(
        &self,
        session_id: &str,
        expected_tool_call_id: Option<&str>,
        answer: String,
        session_scope: bool,
    ) -> Result<RespondAndResumeOutcome, ResponderError> {
        let response_guard = acquire_pending_response_guard(session_id).await;
        let current =
            inspect_pending_response_guarded(&self.ctx.session_repo, session_id, &response_guard)
                .await
                .map_err(map_respond_error)?
                .ok_or(ResponderError::NotFound)?;
        let pending = current
            .pending_question
            .as_ref()
            .ok_or(ResponderError::NoPendingQuestion)?;
        if expected_tool_call_id.is_some_and(|expected| expected != pending.tool_call_id) {
            return Err(ResponderError::PendingQuestionChanged);
        }
        validate_pending_response(pending, &answer).map_err(ResponderError::InvalidResponse)?;

        // 聊天渠道敲进来的 "1"/"2"（Approve/Deny）必须走 typed 提交路径：
        // legacy 文本路径写入的 re-execute 标记不带 generation 绑定，恢复端
        // 会以 "typed permission replay is missing its generation marker"
        // 拒绝重放并直接返回——不产生任何事件、渲染任务永远收不到输出，
        // 会话表现为"选了没反应、卡死到 /new"。这里像 HTTP 决策端点一样
        // 合成结构化 AllowOnce/DenyOnce 回执（授权在响应 CAS 之前安装）。
        let typed_permission = typed_permission_receipt(
            &self.ctx.permission_checker,
            session_id,
            &current,
            pending,
            &answer,
            session_scope,
        );

        let port = ConnectResumePort {
            ctx: self.ctx.clone(),
        };
        let handoff = bamboo_engine::session_app::resume::reserve_response_resume_handoff(
            &port,
            session_id,
            std::time::Duration::from_secs(15),
        )
        .await
        .map_err(|_| {
            ResponderError::Other(
                "the suspending run is still finalizing; answer not consumed".to_string(),
            )
        })?;
        // Subscribe and resolve async configuration before the response CAS.
        // This intentionally gives one response a stable config snapshot;
        // concurrent config changes apply to later requests/runs. After the
        // answer commits, the reserved successor is transferred to a detached
        // owner synchronously, so callback cancellation cannot strand it.
        let receiver = handoff.subscribe();
        let config_snapshot = self.ctx.config.read().await.clone();
        let input = RespondInput {
            session_id: session_id.to_string(),
            user_response: answer,
            model: None,
            model_ref: None,
            provider: None,
            reasoning_effort: None,
        };

        let submission = match typed_permission.as_ref() {
            Ok(Some(receipt)) => {
                submit_pending_permission_response_checked_guarded(
                    &self.ctx.session_repo,
                    input,
                    expected_tool_call_id.map(str::to_string),
                    receipt.clone(),
                    &response_guard,
                )
                .await
            }
            Ok(None) => {
                submit_pending_response_checked_guarded(
                    &self.ctx.session_repo,
                    input,
                    expected_tool_call_id.map(str::to_string),
                    &response_guard,
                )
                .await
            }
            Err(reason) => {
                handoff.abandon().await;
                return Err(ResponderError::InvalidResponse(reason.clone()));
            }
        };
        let (mut session, _submitted_answer, plan_mode_transition, permission_grants) =
            match submission {
                Ok(submission) => submission,
                Err(error) => {
                    handoff.abandon().await;
                    return Err(map_respond_error(error));
                }
            };

        // Mirrors `handlers::agent::respond::handlers::submit`: record any
        // permission grant so the resumed re-execution of the gated tool
        // passes the check without re-prompting. Typed decisions already
        // installed their exact scope/generation before the response CAS.
        let typed_permission_response = matches!(typed_permission, Ok(Some(_)));
        for (perm_type, resource) in &permission_grants {
            if !typed_permission_response {
                if let Some(request_id) = session.metadata.get(PERMISSION_REEXECUTE_METADATA_KEY) {
                    self.ctx.permission_checker.grant_once(
                        session_id,
                        request_id,
                        *perm_type,
                        resource.clone(),
                    );
                }
            }
        }

        // "接下来都允许"（权限问题专属）：把本会话切到 Auto 审批模式，
        // 后续同类/异类操作都不再逐条询问。必须**直存磁盘**（镜像 PATCH
        // 端点）：后续 resume 流程的 save_and_cache 带
        // adopt_fresher_disk_permission_posture 合并，会把"与磁盘不同的
        // 模式"当作过期写入回退掉——只改内存副本会被磁盘旧状态覆盖。
        // 非权限澄清不适用（用户只是在回答模型的问题，没有提权意图）。
        if session_scope && typed_permission_response {
            apply_session_auto_approve(&mut session);
            session.updated_at = chrono::Utc::now();
            if let Err(error) = self.ctx.session_repo.storage().save_session(&session).await {
                tracing::warn!(
                    %session_id,
                    %error,
                    "connect: failed to persist session approval mode"
                );
            }
        }

        if let Some(event) = plan_mode_transition_event(session_id, plan_mode_transition.as_ref()) {
            handoff.publish_event(event);
        }

        let resume_config = resolve_resume_config_snapshot(
            &config_snapshot,
            &self.ctx.provider_registry,
            &session,
            None,
        );

        let outcome = bamboo_engine::session_app::resume::resume_session_execution_with_handoff(
            &port,
            session_id,
            session,
            resume_config,
            handoff,
        )
        .await;
        drop(response_guard);

        match outcome {
            bamboo_engine::session_app::types::ResumeOutcome::Started { .. } => {
                Ok(RespondAndResumeOutcome::Resumed(receiver))
            }
            bamboo_engine::session_app::types::ResumeOutcome::AlreadyRunning { .. } => Ok(
                RespondAndResumeOutcome::NotResumed("this session is already running".to_string()),
            ),
            bamboo_engine::session_app::types::ResumeOutcome::Completed => Ok(
                RespondAndResumeOutcome::NotResumed("nothing left to resume".to_string()),
            ),
            bamboo_engine::session_app::types::ResumeOutcome::NotFound => Ok(
                RespondAndResumeOutcome::NotResumed("session no longer exists".to_string()),
            ),
        }
    }
}

/// 为聊天文本应答（"1"/"2"/"允许"/"拒绝" 映射到 "Approve"/"Deny" 之后）
/// 合成权限决策的结构化回执，与 HTTP 决策端点的 AllowOnce/DenyOnce 处理
/// 一致：先安装 one-shot 授权（绑定 generation），再记录决策、取回执。
/// `session_scope` 为真时（"接下来都允许"类短语）额外安装一个会话范围
/// 的类型化授权，本会话后续同类操作不再询问——本次仍按 AllowOnce 提交。
/// `Ok(None)` 表示当前待答问题不是权限请求（普通澄清），调用方保持
/// legacy 文本提交路径不变；`Err` 表示安装失败，应作为可见错误返回。
#[allow(clippy::too_many_arguments)]
fn typed_permission_receipt(
    checker: &Arc<dyn bamboo_tools::permission::PermissionChecker>,
    session_id: &str,
    session: &bamboo_agent_core::Session,
    pending: &bamboo_agent_core::PendingQuestion,
    answer: &str,
    session_scope: bool,
) -> Result<Option<PermissionDecisionReceipt>, String> {
    let decision_kind = match answer.trim().to_ascii_lowercase().as_str() {
        "approve" | "allow" | "允许" | "同意" | "批准" => PermissionDecisionKind::AllowOnce,
        "deny" | "reject" | "拒绝" | "不允许" => PermissionDecisionKind::DenyOnce,
        _ => return Ok(None),
    };
    let Some(config) = checker.permission_config() else {
        return Ok(None);
    };
    let in_memory = config.pending_request(session_id, &pending.tool_call_id);
    let request = crate::handlers::agent::respond::handlers::pending::resolve_pending_interaction(
        session, pending, in_memory,
    )
    .permission_request
    .filter(|request| request.request_id == pending.tool_call_id);
    let Some(request) = request else {
        return Ok(None);
    };
    // 宿主 Supervisor 会话（默认根编排会话）上的审批必须走正式 typed
    // 决策端点：其重放绑定要求正式安装 supervisor 授权，聊天文本应答
    // 不得代为铸造回执（supervisor_approval_tests::
    // connect_supervisor_text_answer_cannot_replace_a_typed_receipt）。
    // 普通用户会话（聊天渠道桥接创建的会话）没有该权威记录，不受限。
    let supervisor_bound = session
        .messages
        .iter()
        .rev()
        .find(|message| message.tool_call_id.as_deref() == Some(pending.tool_call_id.as_str()))
        .and_then(|message| message.metadata.as_ref())
        .and_then(|metadata| {
            metadata.get(
                bamboo_agent_core::tools::ExecutingSupervisorObservation::PERMISSION_REPLAY_METADATA_KEY,
            )
        })
        .is_some();
    if supervisor_bound {
        return Ok(None);
    }
    if !request.allowed_decisions.contains(&decision_kind) {
        return Err(format!(
            "decision {decision_kind:?} is not allowed for this request"
        ));
    }
    if decision_kind == PermissionDecisionKind::AllowOnce {
        config.grant_once_for_generation(
            session_id,
            &request.request_id,
            &request.request_generation,
            request.permission_type,
            request.resource.clone(),
        )?;
    }
    if session_scope {
        // 会话级持续授权（"接下来都允许"）：从建议 matcher 里取最宽的
        // 一个（conservative_matchers 按窄→宽排序，末位是命令前缀/目录
        // 子树/HTTP origin），没有建议时退回精确资源匹配。安装失败不
        // 阻塞本次批准——本次仍按 AllowOnce 正常放行。
        let matcher = request
            .suggested_matchers
            .iter()
            .rev()
            .find(|matcher| matcher.validate(request.permission_type).is_ok())
            .cloned()
            .unwrap_or(PermissionMatcher {
                id: "exact_resource".to_string(),
                kind: PermissionMatcherKind::ExactResource,
                value: request.resource.clone(),
            });
        if let Err(error) = config.grant_typed_scoped_session_permission(
            session_id,
            request.permission_type,
            matcher,
        ) {
            tracing::warn!("connect: session-scope permission grant failed: {error}");
        }
    }
    let decision = PermissionDecision {
        request_id: request.request_id.clone(),
        request_generation: request.request_generation.clone(),
        decision: decision_kind,
        matcher_id: None,
        expected_policy_revision: None,
        confirm_global: false,
    };
    config.record_decision(session_id, decision.clone())?;
    match config.decision_receipt(
        session_id,
        &decision.request_id,
        &decision.request_generation,
    ) {
        Some(receipt) => Ok(Some(receipt)),
        None => Err("permission decision receipt was not recorded".to_string()),
    }
}

/// "都允许"的会话级开关：把本会话的请求权限模式切到 Auto——引擎的
/// per-session 机制（`ToolExecutionSessionFlags::from_session_and_configured_mode`
/// 读取），后续本会话的工具调用抑制审批提示；硬拒绝（危险命令、平台/
/// 持久化拒绝规则）与计划模式边界仍然生效。/new 或会话轮换后回到默认。
/// 镜像 `handlers::agent::sessions::handlers::crud::patch` 的权限模式变更
/// （typed 模式 + legacy 镜像原子更新 + 审计元数据 + metadata_version）。
pub(crate) fn apply_session_auto_approve(session: &mut bamboo_agent_core::Session) {
    use bamboo_domain::SessionPermissionMode;
    apply_session_permission_mode(session, SessionPermissionMode::Auto);
}

/// 切换本会话的请求权限模式（Auto = 不再逐条询问；Default = 恢复），
/// 记录审计元数据并提升 `metadata_version`，使运行中回合的磁盘审计
/// 刷新（`fresher_disk_permission_audit`）能在下一回合边界生效。
pub(crate) fn apply_session_permission_mode(
    session: &mut bamboo_agent_core::Session,
    mode: bamboo_domain::SessionPermissionMode,
) {
    use bamboo_domain::{AgentRuntimeState, PermissionAuditSeed};
    let runtime = session
        .agent_runtime_state
        .get_or_insert_with(AgentRuntimeState::default);
    if runtime.effective_permission_mode() == mode {
        return;
    }
    runtime.set_permission_mode(mode);
    let configured = bamboo_domain::PermissionMode::Default;
    let resolution = bamboo_domain::resolve_permission_mode(mode, configured);
    let transitioned_at = chrono::Utc::now().to_rfc3339();
    let _ = bamboo_domain::record_permission_audit(
        &mut session.metadata,
        &PermissionAuditSeed::bamboo_runtime(0, resolution),
        Some(transitioned_at.as_str()),
    );
    session.metadata_version = session.metadata_version.saturating_add(1);
}

/// 单独发送的"接下来都允许"类消息（不是对审批弹窗的回复）也当作会话
/// 指令处理：消息很短且整体就是持续授权/恢复确认短语。`Some(true)` 切
/// Auto，`Some(false)` 恢复逐条确认，`None` 不是指令（进正常消息流）。
pub fn standalone_session_scope_command(text: &str) -> Option<bool> {
    let trimmed = text.trim();
    if trimmed.chars().count() > 12 {
        return None;
    }
    if wants_session_scope(trimmed) {
        return Some(true);
    }
    const RESTORE_PHRASES: &[&str] = &[
        "恢复确认",
        "恢复询问",
        "重新询问",
        "恢复逐条",
        "每次都问",
        "恢复默认",
        "ask again",
    ];
    let lower = trimmed.to_lowercase();
    if RESTORE_PHRASES.iter().any(|phrase| lower.contains(phrase)) {
        return Some(false);
    }
    None
}

fn plan_mode_transition_event(
    session_id: &str,
    transition: Option<&bamboo_engine::session_app::respond::PlanModeTransition>,
) -> Option<AgentEvent> {
    use bamboo_engine::session_app::respond::PlanModeTransition;
    transition.map(|transition| match transition {
        PlanModeTransition::Entered {
            reason,
            pre_permission_mode,
            entered_at,
            status,
            plan_file_path,
        } => AgentEvent::PlanModeEntered {
            session_id: session_id.to_string(),
            reason: reason.clone(),
            pre_permission_mode: pre_permission_mode.clone(),
            entered_at: *entered_at,
            status: *status,
            plan_file_path: plan_file_path.clone(),
        },
        PlanModeTransition::Exited {
            approved,
            restored_mode,
            plan,
        } => AgentEvent::PlanModeExited {
            session_id: session_id.to_string(),
            approved: *approved,
            restored_mode: restored_mode.clone(),
            plan: plan.clone(),
        },
    })
}

// ---------------------------------------------------------------------------
// ConnectResumePort — ResumeExecutionPort for connect-bridged sessions
// ---------------------------------------------------------------------------

/// [`ResumeExecutionPort`] backed by [`ConnectContext`] instead of `AppState`
/// — the connect-scoped counterpart of the server's
/// `app_state::resume_adapter::AppStateResumeRef`. Spawns through the
/// crate-agnostic `spawn_session_execution` (matching
/// `bridge::ConnectBridge::run_prompt`'s fresh-prompt spawn exactly: same
/// tools/agent/model-roster resolution, no guardian/bash-resume-hook — those
/// remain a later phase, same as the fresh-prompt path), rather than the
/// server handler layer's `spawn_agent_execution` (which pulls in
/// `AppState`-specific wiring connect deliberately doesn't use).
struct ConnectResumePort {
    ctx: ConnectContext,
}

#[async_trait::async_trait]
impl ResumeExecutionPort for ConnectResumePort {
    async fn load_session(&self, session_id: &str) -> Option<Session> {
        self.ctx.session_repo.load_merged(session_id).await
    }

    async fn save_and_cache_session(&self, session: &mut Session) {
        self.ctx.session_repo.save_and_cache(session).await;
    }

    async fn reserve_session_execution(
        &self,
        session_id: &str,
        event_sender: &broadcast::Sender<AgentEvent>,
    ) -> SessionExecutionReserveOutcome {
        reserve_session_execution(
            &self.ctx.agent,
            &self.ctx.agent_runners,
            &self.ctx.session_event_senders,
            session_id,
            event_sender,
        )
        .await
    }

    async fn get_or_create_event_sender(&self, session_id: &str) -> broadcast::Sender<AgentEvent> {
        get_or_create_event_sender(&self.ctx.session_event_senders, session_id).await
    }

    fn dispatch_resume_execution(
        &self,
        request: ResumeSpawnRequest,
    ) -> Result<(), ResumeSpawnRequest> {
        let owner = ConnectResumePort {
            ctx: self.ctx.clone(),
        };
        tokio::spawn(async move {
            ResumeExecutionPort::spawn_resume_execution(&owner, request).await;
        });
        Ok(())
    }

    async fn spawn_resume_execution(&self, request: ResumeSpawnRequest) {
        let ResumeSpawnRequest {
            session_id,
            mut session,
            mut execution_reservation,
            event_sender,
            config,
        } = request;
        if let Err(error) = execution_reservation.ensure_registered().await {
            tracing::warn!(
                %session_id,
                run_id = %execution_reservation.run_id(),
                %error,
                "cannot resume connect session without exact router ownership"
            );
            return;
        }

        let model = session.model.clone();
        let reasoning_effort = session.reasoning_effort;
        let model_roster = ModelRoster {
            model: Some(model),
            provider_name: Some(config.provider_name.clone()),
            provider_type: config.provider_type.clone(),
            fast: RoleModel::from_parts(config.fast_model.clone(), None),
            background: RoleModel::from_parts(
                config.background_model.clone(),
                config.background_model_provider.clone(),
            ),
            summarization: RoleModel::from_parts(
                config.summarization_model.clone(),
                config.summarization_model_provider.clone(),
            ),
        };

        let (mpsc_tx, _forwarder_handle, history_commit_barrier) =
            create_event_forwarder_with_history_commit_barrier(
                session_id.clone(),
                execution_reservation.run_id().to_string(),
                event_sender,
                self.ctx.agent_runners.clone(),
                self.ctx.account_feed_inbox.clone(),
            );

        // If the user just approved a permission prompt, the gated tool call
        // was intercepted before it ran — its recorded result is only a
        // placeholder. Re-execute it for real now (the grant was already
        // applied to `ctx.permission_checker` in `EngineResponder`), write
        // the output back, then start the loop — mirrors
        // `app_state::resume_adapter::AppStateResumeRef::spawn_resume_execution`
        // exactly, minus the `AppState`-specific plumbing.
        let reexecute_tool_call_id = session
            .metadata
            .get(PERMISSION_REEXECUTE_METADATA_KEY)
            .cloned();
        let reexecute_request_generation = session
            .metadata
            .get(PERMISSION_REEXECUTE_GENERATION_METADATA_KEY)
            .cloned();

        let Some(reexecute_tool_call_id) = reexecute_tool_call_id else {
            if reexecute_request_generation.is_some() {
                tracing::error!(
                    %session_id,
                    "connect found orphaned permission replay generation marker; refusing to resume"
                );
                refuse_resume(
                    &mpsc_tx,
                    "内部状态异常（权限重放标记孤立），本轮已停止；重新发送消息即可继续"
                        .to_string(),
                )
                .await;
                return;
            }
            consume_pending_clarification_resume(&mut session);
            spawn_session_execution(SessionExecutionArgs {
                agent: self.ctx.agent.clone(),
                session_id,
                session,
                execution_reservation,
                tools_override: Some(self.ctx.tools.clone()),
                provider_override: None,
                model_roster,
                reasoning_effort,
                reasoning_effort_source: "connect_resume".to_string(),
                auxiliary_model_resolver: None,
                disabled_filter_resolver: None,
                disabled_tools: Some(config.disabled_tools.clone()),
                disabled_skill_ids: Some(config.disabled_skill_ids.clone()),
                selected_skill_ids: None,
                selected_skill_mode: None,
                mpsc_tx,
                history_commit_barrier,
                image_fallback: config.image_fallback.clone(),
                gold_config: config.gold_config.clone(),
                guardian_config: None,
                guardian_spawner: None,
                bash_resume_hook: None,
                bash_completion_sink: None,
                app_data_dir: self.ctx.app_data_dir.clone(),
                // No per-request override on this path; the config-level
                // default (issue #221) still applies.
                run_budget: None,
                runners: self.ctx.agent_runners.clone(),
                sessions_cache: self.ctx.session_repo.cache().clone(),
                on_complete: None,
                // Connect drives root sessions; a child finishing on this
                // path is backstopped by the child-wait watchdog (#546).
                child_completion_handler: None,
            });
            return;
        };

        let ctx = self.ctx.clone();
        tokio::spawn(async move {
            let mut session = session;

            if let Some(replay_target) = find_pending_tool_call(
                &session,
                &reexecute_tool_call_id,
                reexecute_request_generation.as_deref(),
            ) {
                if reexecute_request_generation.is_none()
                    && replay_target.request_generation().is_some()
                {
                    tracing::error!(
                        %session_id,
                        tool_call_id = %reexecute_tool_call_id,
                        "connect typed permission replay is missing its generation marker; refusing to resume"
                    );
                    refuse_resume(
                        &mpsc_tx,
                        "权限审批记录不完整，本轮已停止；重新发送消息即可继续".to_string(),
                    )
                    .await;
                    return;
                }
                let tool_call = replay_target.tool_call().clone();
                let tool_name = tool_call.function.name.clone();
                let executor = ctx.tools.clone();
                let replay_owner = bamboo_domain::resolve_tool_reference_name(&tool_name, |name| {
                    executor.owns_exact_tool(name)
                });
                if replay_owner.is_none() && reexecute_request_generation.is_some() {
                    tracing::error!(%session_id, %tool_name, "connect approved replay has no registered execution owner; markers retained");
                    refuse_resume(
                        &mpsc_tx,
                        format!("审批通过的工具「{tool_name}」当前不可用，本轮已停止"),
                    )
                    .await;
                    return;
                }
                let executing_supervisor = match validate_permission_replay_authority(
                    &session,
                    &replay_target,
                    replay_owner.as_deref().unwrap_or(&tool_name),
                ) {
                    Ok(observation) => observation,
                    Err(error) => {
                        tracing::error!(%session_id, %error, "Supervisor approval replay binding failed closed");
                        refuse_resume(&mpsc_tx, format!("审批重放校验失败，本轮已停止（{error}）"))
                            .await;
                        return;
                    }
                };
                let configured_mode = ctx
                    .permission_checker
                    .permission_config()
                    .map(|config| config.mode())
                    .unwrap_or_default();
                let decision = match refresh_approval_replay_posture(
                    ctx.session_repo.storage().as_ref(),
                    &mut session,
                    configured_mode,
                    replay_owner.as_deref(),
                )
                .await
                {
                    Ok(decision) => decision,
                    Err(error) => {
                        tracing::error!(
                            %session_id,
                            tool_call_id = %reexecute_tool_call_id,
                            %error,
                            "connect approval replay posture refresh failed closed"
                        );
                        refuse_resume(&mpsc_tx, format!("权限状态刷新失败，本轮已停止（{error}）"))
                            .await;
                        return;
                    }
                };
                let blocked_by_tool_authority = matches!(
                    decision,
                    ApprovalReplayDecision::BlockedByRootToolAuthority
                        | ApprovalReplayDecision::BlockedByUnavailableTool
                );
                session.metadata.remove(PERMISSION_REEXECUTE_METADATA_KEY);
                session
                    .metadata
                    .remove(PERMISSION_REEXECUTE_GENERATION_METADATA_KEY);

                let (content, success) = match decision {
                    ApprovalReplayDecision::BlockedByPlan(_) => (
                        format!(
                            "Plan mode blocked approved mutating tool '{tool_name}'; the stale approval was not executed"
                        ),
                        false,
                    ),
                    ApprovalReplayDecision::BlockedByRootToolAuthority => (
                        format!(
                            "Root orchestration policy blocked approved tool '{tool_name}'; the stale approval was not executed"
                        ),
                        false,
                    ),
                    ApprovalReplayDecision::BlockedByUnavailableTool => (
                        format!(
                            "Approved tool '{tool_name}' is no longer available; the stale approval was not executed"
                        ),
                        false,
                    ),
                    ApprovalReplayDecision::Execute(flags) => {
                        let replay_owner = replay_owner
                            .as_deref()
                            .expect("Execute requires a registered execution owner");
                        let Some(permission_config) =
                            ctx.permission_checker.permission_config()
                        else {
                            tracing::error!(
                                %session_id,
                                tool_call_id = %reexecute_tool_call_id,
                                "connect typed approval replay has no permission configuration; refusing to resume"
                            );
                            refuse_resume(
                                &mpsc_tx,
                                "权限配置不可用，已批准的操作无法继续执行".to_string(),
                            )
                            .await;
                            return;
                        };
                        if let Err(error) = restore_permission_replay_authorization(
                            permission_config.as_ref(),
                            &session,
                            &replay_target,
                            replay_owner,
                        ) {
                            tracing::error!(
                                %session_id,
                                tool_call_id = %reexecute_tool_call_id,
                                %error,
                                "connect typed approval replay authorization recovery failed closed"
                            );
                            refuse_resume(
                                &mpsc_tx,
                                format!("审批授权恢复失败，本轮已停止（{error}）"),
                            )
                            .await;
                            return;
                        }
                        let is_mutating = bamboo_tools::orchestrator::classify_tool(&tool_name)
                            == bamboo_tools::orchestrator::ToolMutability::Mutating;
                        let mut emitter = bamboo_tools::ToolEmitter::new(
                            &tool_call.id,
                            &tool_name,
                            is_mutating,
                        );
                        emitter.set_auto_approved(true);
                        let _ = mpsc_tx
                            .send(emitter.begin().clone().into_agent_event())
                            .await;
                        let exec_result = bamboo_tools::permission::with_permission_replay_generation(
                            session.id.as_str(),
                            reexecute_tool_call_id.as_str(),
                            reexecute_request_generation.as_deref(),
                            executor.execute_exact_with_context_outcome(
                                &tool_call,
                                replay_owner,
                                ToolExecutionContext {
                                    executing_supervisor,
                                    session_id: Some(session.id.as_str()),
                                    root_session_id: Some(
                                        if session.root_session_id.trim().is_empty() {
                                            session.id.as_str()
                                        } else {
                                            session.root_session_id.as_str()
                                        },
                                    ),
                                    tool_call_id: reexecute_tool_call_id.as_str(),
                                    event_tx: Some(&mpsc_tx),
                                    available_tool_schemas: None,
                                    bypass_permissions: flags.bypass_permissions,
                                    auto_approve_permissions: flags.auto_approve_permissions,
                                    plan_read_only: flags.plan_read_only,
                                    can_async_resume: false,
                                    bash_completion_sink: None,
                                    pre_parsed_args: None,
                                },
                            ),
                        )
                        .await.map(bamboo_agent_core::tools::ToolOutcome::into_tool_result);

                        match exec_result {
                            Ok(tool_result) => {
                                match repark_permission_replay(
                                    &mut session,
                                    &replay_target,
                                    &tool_result,
                                    replay_owner,
                                ) {
                                    Ok(Some(reparked)) => {
                                        let _ = mpsc_tx
                                            .send(
                                                emitter
                                                    .finish(Some(
                                                        "Awaiting additional permission approval"
                                                            .to_string(),
                                                    ))
                                                    .clone()
                                                    .into_agent_event(),
                                            )
                                            .await;
                                        let _ = mpsc_tx
                                            .send(AgentEvent::ToolComplete {
                                                tool_call_id: tool_call.id.clone(),
                                                result: tool_result,
                                            })
                                            .await;
                                        let _ = mpsc_tx
                                            .send(AgentEvent::NeedClarification {
                                                question: reparked.question,
                                                options: (!reparked.options.is_empty())
                                                    .then_some(reparked.options),
                                                tool_call_id: Some(tool_call.id.clone()),
                                                tool_name: Some(tool_name.clone()),
                                                allow_custom: reparked.allow_custom,
                                                source: Some(
                                                    bamboo_agent_core::PendingQuestionSource::PauseTool,
                                                ),
                                            })
                                            .await;
                                        ctx.session_repo.save_and_cache(&mut session).await;
                                        return;
                                    }
                                    Ok(None) => {}
                                    Err(error) => {
                                        tracing::error!(
                                            %session_id,
                                            tool_call_id = %reexecute_tool_call_id,
                                            %error,
                                            "connect additional permission replay could not be re-parked; refusing to resume"
                                        );
                                        refuse_resume(
                                            &mpsc_tx,
                                            format!("后续权限审批写入失败，本轮已停止（{error}）"),
                                        )
                                        .await;
                                        return;
                                    }
                                }
                                let _ = mpsc_tx
                                    .send(
                                        emitter
                                            .finish(Some(
                                                "Re-executed after approval".to_string(),
                                            ))
                                            .clone()
                                            .into_agent_event(),
                                    )
                                    .await;
                                let _ = mpsc_tx
                                    .send(AgentEvent::ToolComplete {
                                        tool_call_id: tool_call.id.clone(),
                                        result: tool_result.clone(),
                                    })
                                    .await;
                                (tool_result.result, tool_result.success)
                            }
                            Err(error) => {
                                let message =
                                    format!("Tool re-execution after approval failed: {error}");
                                let _ = mpsc_tx
                                    .send(
                                        emitter.error(message.clone()).clone().into_agent_event(),
                                    )
                                    .await;
                                (message, false)
                            }
                        }
                    }
                };

                tracing::info!(
                    "[{}] connect: resolved approved tool replay '{}' ({}) -> success={}",
                    session_id,
                    tool_name,
                    reexecute_tool_call_id,
                    success
                );
                if !apply_tool_result(&mut session, &replay_target, content, success) {
                    tracing::error!(
                        %session_id,
                        tool_call_id = %reexecute_tool_call_id,
                        "connect approved tool replay result target changed unexpectedly; refusing to resume"
                    );
                    refuse_resume(
                        &mpsc_tx,
                        "会话状态与审批记录不一致，本轮已停止；重新发送消息即可继续".to_string(),
                    )
                    .await;
                    return;
                }
                if blocked_by_tool_authority {
                    if let Err(error) = ctx.session_repo.save_replay_resolution(&mut session).await
                    {
                        tracing::error!(%session_id, %error, "connect blocked approval replay result failed to persist; refusing to resume");
                        refuse_resume(&mpsc_tx, format!("审批结果保存失败，本轮已停止（{error}）"))
                            .await;
                        return;
                    }
                } else {
                    ctx.session_repo.save_and_cache(&mut session).await;
                }
            } else {
                tracing::error!(
                    %session_id,
                    tool_call_id = %reexecute_tool_call_id,
                    request_generation = ?reexecute_request_generation,
                    "connect permission replay target missing or generation-mismatched; markers retained and resume refused"
                );
                refuse_resume(
                    &mpsc_tx,
                    "找不到待重放的审批目标（可能已被新的提问取代），本轮已停止".to_string(),
                )
                .await;
                return;
            }

            consume_pending_clarification_resume(&mut session);
            spawn_session_execution(SessionExecutionArgs {
                agent: ctx.agent.clone(),
                session_id,
                session,
                execution_reservation,
                tools_override: Some(ctx.tools.clone()),
                provider_override: None,
                model_roster,
                reasoning_effort,
                reasoning_effort_source: "connect_resume".to_string(),
                auxiliary_model_resolver: None,
                disabled_filter_resolver: None,
                disabled_tools: Some(config.disabled_tools.clone()),
                disabled_skill_ids: Some(config.disabled_skill_ids.clone()),
                selected_skill_ids: None,
                selected_skill_mode: None,
                mpsc_tx,
                history_commit_barrier,
                image_fallback: config.image_fallback.clone(),
                gold_config: config.gold_config.clone(),
                guardian_config: None,
                guardian_spawner: None,
                bash_resume_hook: None,
                bash_completion_sink: None,
                app_data_dir: ctx.app_data_dir.clone(),
                // No per-request override on this path; the config-level
                // default (issue #221) still applies.
                run_budget: None,
                runners: ctx.agent_runners.clone(),
                sessions_cache: ctx.session_repo.cache().clone(),
                on_complete: None,
                // Connect drives root sessions; a child finishing on this
                // path is backstopped by the child-wait watchdog (#546).
                child_completion_handler: None,
            });
        });
    }
}

/// Find the concrete approved invocation, newest-first and generation-bound.
fn find_pending_tool_call(
    session: &Session,
    tool_call_id: &str,
    request_generation: Option<&str>,
) -> Option<PermissionReplayTarget> {
    find_permission_replay_target(session, tool_call_id, request_generation)
}

/// Overwrite only the exact generation-bound tool-result message.
fn apply_tool_result(
    session: &mut Session,
    target: &PermissionReplayTarget,
    content: String,
    success: bool,
) -> bool {
    apply_permission_replay_result(session, target, content, success)
}

#[cfg(test)]
mod tests {
    use bamboo_agent_core::tools::{FunctionCall, ToolCall};
    use bamboo_agent_core::Message;

    use super::*;

    fn supervisor_context(state: &crate::app_state::AppState) -> ConnectContext {
        ConnectContext {
            agent: state.agent.clone(),
            tools: state.tools_for(crate::tools::ToolSurface::Root),
            session_repo: state.session_repo.clone(),
            agent_runners: state.agent_runners.clone(),
            session_event_senders: state.session_event_senders.clone(),
            account_feed_inbox: None,
            app_data_dir: Some(state.app_data_dir.clone()),
            config: state.config.clone(),
            provider_registry: state.provider_registry.clone(),
            project_store: state.project_store.clone(),
            workspace_resolver: state.workspace_resolver.clone(),
            project_ids_by_platform: Arc::new(Default::default()),
            permission_checker: state.permission_checker.clone(),
        }
    }

    #[tokio::test]
    async fn connect_supervisor_typed_replay_restores_identity_and_rejects_corruption() {
        use crate::app_state::resume_adapter::supervisor_tests::Fixture;
        for corrupt in [false, true] {
            let fixture = Box::pin(Fixture::pending()).await;
            fixture.prepare_workspace_catalog().await;
            fixture.formal_approve().await;
            if corrupt {
                fixture.corrupt().await;
            }
            let ctx = supervisor_context(&fixture.state);
            let session = fixture.reload().await;
            let config = resolve_resume_config_snapshot(
                &*ctx.config.read().await,
                &ctx.provider_registry,
                &session,
                None,
            );
            let outcome = bamboo_engine::session_app::resume::resume_session_execution(
                &ConnectResumePort { ctx },
                &session.id,
                config,
            )
            .await;
            assert!(matches!(
                outcome,
                bamboo_engine::session_app::types::ResumeOutcome::Started { .. }
            ));
            fixture.settled(usize::from(!corrupt), corrupt).await;
        }
    }

    #[tokio::test]
    async fn connect_resume_consumes_approved_call_after_root_tool_tightening() {
        use crate::app_state::resume_adapter::supervisor_tests::{Fixture, CALL};
        let fixture = Box::pin(Fixture::pending()).await;
        fixture.prepare_workspace_catalog().await;
        fixture.formal_approve().await;
        let mut selected = fixture.reload().await;
        selected.set_root_orchestration_only(true).unwrap();
        fixture.state.storage.save_session(&selected).await.unwrap();

        let ctx = supervisor_context(&fixture.state);
        let session = fixture.reload().await;
        let config = resolve_resume_config_snapshot(
            &*ctx.config.read().await,
            &ctx.provider_registry,
            &session,
            None,
        );
        let outcome = bamboo_engine::session_app::resume::resume_session_execution(
            &ConnectResumePort { ctx },
            &session.id,
            config,
        )
        .await;
        assert!(matches!(
            outcome,
            bamboo_engine::session_app::types::ResumeOutcome::Started { .. }
        ));
        fixture.settled(0, false).await;
        let saved = fixture.reload().await;
        let result = saved
            .messages
            .iter()
            .rev()
            .find(|message| message.tool_call_id.as_deref() == Some(CALL))
            .unwrap();
        assert_eq!(result.tool_success, Some(false));
        assert!(result.content.contains("Root orchestration policy blocked"));
    }

    #[tokio::test]
    async fn connect_supervisor_text_answer_cannot_replace_a_typed_receipt() {
        use crate::app_state::resume_adapter::supervisor_tests::{Fixture, CALL};
        let fixture = Box::pin(Fixture::pending()).await;
        let responder = EngineResponder::new(supervisor_context(&fixture.state));
        let outcome = responder
            .respond_and_resume(
                &fixture.original.session_id,
                Some(CALL),
                "Approve".into(),
                false,
            )
            .await
            .unwrap();
        assert!(matches!(outcome, RespondAndResumeOutcome::Resumed(_)));
        fixture.settled(0, true).await;
        let session = fixture.reload().await;
        let result = session
            .messages
            .iter()
            .rev()
            .find(|message| message.tool_call_id.as_deref() == Some(CALL))
            .unwrap();
        assert!(result
            .metadata
            .as_ref()
            .unwrap()
            .get("permission_decision_receipt")
            .is_none());
    }

    #[test]
    fn typed_permission_receipt_synthesizes_generation_bound_receipt() {
        use bamboo_agent_core::PendingQuestionSource;
        use bamboo_tools::permission::{
            ConfigPermissionChecker, PermissionConfig, PermissionDecisionKind,
            PermissionReasonCode, PermissionRequest, PermissionType, RiskLevel,
        };

        // 普通用户会话（非 supervisor 会话）：文本 "Approve"/"允许"/"拒绝"
        // 必须合成带 generation 绑定的 typed 回执并安装授权——这是聊天渠道
        // 审批 "选了没反应、卡死" 死锁（missing generation marker）的回归
        // 测试。权限请求是一次性的（决策即消费），每个用例全新注册。
        let fresh_setup = || {
            let config = Arc::new(PermissionConfig::new());
            config.register_pending_request(PermissionRequest {
                request_id: "call-1".to_string(),
                request_generation: "generation-7".to_string(),
                session_id: "session".to_string(),
                workspace_path: None,
                tool_name: "Bash".to_string(),
                permission_type: PermissionType::ExecuteCommand,
                resource: "current-command".to_string(),
                operation_summary: "run a command".to_string(),
                risk_level: RiskLevel::Medium,
                reason_code: PermissionReasonCode::ConfiguredAlwaysAsk,
                effective_mode: bamboo_domain::PermissionMode::Default,
                bypass_requested: false,
                auto_approve_requested: false,
                policy_revision: 0,
                matched_rule: None,
                allowed_decisions: vec![
                    PermissionDecisionKind::AllowOnce,
                    PermissionDecisionKind::DenyOnce,
                ],
                suggested_matchers: Vec::new(),
            });
            let checker: Arc<dyn bamboo_tools::permission::PermissionChecker> =
                Arc::new(ConfigPermissionChecker::new(config.clone()));
            let pending = bamboo_agent_core::PendingQuestion {
                tool_call_id: "call-1".to_string(),
                tool_name: "Bash".to_string(),
                question: "Permission required".to_string(),
                options: vec!["Approve".to_string(), "Deny".to_string()],
                allow_custom: false,
                source: PendingQuestionSource::PauseTool,
            };
            let session = Session::new("session", "model");
            (config, checker, session, pending)
        };

        for (answer, expected_kind) in [
            ("Approve", PermissionDecisionKind::AllowOnce),
            ("允许", PermissionDecisionKind::AllowOnce),
            ("拒绝", PermissionDecisionKind::DenyOnce),
        ] {
            let (_config, checker, session, pending) = fresh_setup();
            let receipt =
                typed_permission_receipt(&checker, "session", &session, &pending, answer, false)
                    .expect("receipt synthesis succeeds")
                    .expect("permission ask yields a typed receipt");
            assert_eq!(receipt.session_id, "session");
            assert_eq!(receipt.decision.request_id, "call-1");
            assert_eq!(receipt.decision.request_generation, "generation-7");
            assert_eq!(receipt.decision.decision, expected_kind);
        }

        // 非权限词表之外的答案不是权限决策，回落 legacy 路径。
        let (_config, checker, session, pending) = fresh_setup();
        assert!(
            typed_permission_receipt(&checker, "session", &session, &pending, "Banana", false)
                .expect("no error")
                .is_none()
        );

        // 合成授权后，generation 绑定的 one-shot 授权对重放校验可见。
        let (config, checker, session, pending) = fresh_setup();
        let receipt =
            typed_permission_receipt(&checker, "session", &session, &pending, "Approve", false)
                .expect("receipt synthesis succeeds")
                .expect("permission ask yields a typed receipt");
        assert_eq!(receipt.decision.decision, PermissionDecisionKind::AllowOnce);
        assert!(config
            .decision_receipt("session", "call-1", "generation-7")
            .is_some());
        // 单次批准不安装会话级授权。
        assert!(!config.is_scoped_session_granted(
            "session",
            bamboo_tools::permission::PermissionType::ExecuteCommand,
            "current-command"
        ));
    }

    #[test]
    fn typed_permission_receipt_session_scope_installs_session_grant() {
        use bamboo_tools::permission::PermissionType;

        // "接下来都允许"：本次仍是 AllowOnce 回执，同时安装会话级授权，
        // 本会话后续同类资源（无建议 matcher 时精确匹配）不再询问。
        let (config, checker, session, pending) = session_scope_setup();
        let receipt =
            typed_permission_receipt(&checker, "session", &session, &pending, "Approve", true)
                .expect("receipt synthesis succeeds")
                .expect("permission ask yields a typed receipt");
        assert_eq!(receipt.decision.decision, PermissionDecisionKind::AllowOnce);
        assert!(config.is_scoped_session_granted(
            "session",
            PermissionType::ExecuteCommand,
            "current-command"
        ));
    }

    #[test]
    fn standalone_session_scope_command_detects_phrases() {
        assert_eq!(standalone_session_scope_command("接下来都允许"), Some(true));
        assert_eq!(standalone_session_scope_command("都允许"), Some(true));
        assert_eq!(standalone_session_scope_command("恢复确认"), Some(false));
        assert_eq!(standalone_session_scope_command("每次都问"), Some(false));
        // 长消息里包含短语不算指令，正常进消息流。
        assert_eq!(
            standalone_session_scope_command("帮我看看这个问题：总是允许执行吗"),
            None
        );
        assert_eq!(standalone_session_scope_command("普通消息"), None);
    }

    #[test]
    fn wants_session_scope_matches_persistent_allow_phrases() {
        for text in [
            "接下来都允许",
            "都允许",
            "全部允许",
            "以后都允许",
            "总是允许",
            "不再询问了",
            "别问了",
            "always allow",
        ] {
            assert!(wants_session_scope(text), "should match: {text}");
        }
        for text in [
            "以后都不允许",
            "全部拒绝",
            "不允许",
            "1",
            "允许",
            "拒绝",
            "帮我写个脚本，总是允许执行的那种说明文字很长很长很长很长很长很长很长很长很长很长很长很长很长很长",
        ] {
            assert!(!wants_session_scope(text), "should not match: {text}");
        }
    }

    #[test]
    fn match_text_answer_maps_session_scope_phrase_to_affirmative() {
        let pending = ask(vec!["Approve", "Deny"], false);
        assert_eq!(
            match_text_answer(&pending, "接下来都允许").as_deref(),
            Some("Approve")
        );
        // OPEN 问题自由文本优先，不劫持为选项。
        let open = ask(vec!["A", "B"], true);
        assert_eq!(
            match_text_answer(&open, "接下来都允许").as_deref(),
            Some("接下来都允许")
        );
    }

    #[test]
    fn apply_session_auto_approve_switches_session_mode() {
        use bamboo_agent_core::tools::ToolExecutionSessionFlags;
        use bamboo_domain::SessionPermissionMode;

        // "都允许"应把本会话切到 Auto：后续工具调用的会话旗标解析为
        // auto_approve（抑制审批提示），且幂等（重复调用不叠加）。
        let mut session = Session::new("session", "model");
        let before = session.metadata_version;
        apply_session_auto_approve(&mut session);
        let requested = session
            .agent_runtime_state
            .as_ref()
            .map(|state| state.effective_permission_mode());
        assert_eq!(requested, Some(SessionPermissionMode::Auto));
        assert!(session.metadata_version > before);
        let flags = ToolExecutionSessionFlags::from_session_and_configured_mode(
            &session,
            Default::default(),
        );
        assert!(flags.auto_approve_permissions);
        assert!(!flags.plan_read_only);

        let version_after_first = session.metadata_version;
        apply_session_auto_approve(&mut session);
        assert_eq!(session.metadata_version, version_after_first);
    }

    /// 与 `typed_permission_receipt_synthesizes_generation_bound_receipt`
    /// 的 fresh_setup 相同，仅隔离复用。
    fn session_scope_setup() -> (
        Arc<bamboo_tools::permission::PermissionConfig>,
        Arc<dyn bamboo_tools::permission::PermissionChecker>,
        Session,
        bamboo_agent_core::PendingQuestion,
    ) {
        use bamboo_agent_core::PendingQuestionSource;
        use bamboo_tools::permission::{
            ConfigPermissionChecker, PermissionConfig, PermissionDecisionKind,
            PermissionReasonCode, PermissionRequest, PermissionType, RiskLevel,
        };
        let config = Arc::new(PermissionConfig::new());
        config.register_pending_request(PermissionRequest {
            request_id: "call-1".to_string(),
            request_generation: "generation-7".to_string(),
            session_id: "session".to_string(),
            workspace_path: None,
            tool_name: "Bash".to_string(),
            permission_type: PermissionType::ExecuteCommand,
            resource: "current-command".to_string(),
            operation_summary: "run a command".to_string(),
            risk_level: RiskLevel::Medium,
            reason_code: PermissionReasonCode::ConfiguredAlwaysAsk,
            effective_mode: bamboo_domain::PermissionMode::Default,
            bypass_requested: false,
            auto_approve_requested: false,
            policy_revision: 0,
            matched_rule: None,
            allowed_decisions: vec![
                PermissionDecisionKind::AllowOnce,
                PermissionDecisionKind::DenyOnce,
            ],
            suggested_matchers: Vec::new(),
        });
        let checker: Arc<dyn bamboo_tools::permission::PermissionChecker> =
            Arc::new(ConfigPermissionChecker::new(config.clone()));
        let pending = bamboo_agent_core::PendingQuestion {
            tool_call_id: "call-1".to_string(),
            tool_name: "Bash".to_string(),
            question: "Permission required".to_string(),
            options: vec!["Approve".to_string(), "Deny".to_string()],
            allow_custom: false,
            source: PendingQuestionSource::PauseTool,
        };
        let session = Session::new("session", "model");
        (config, checker, session, pending)
    }

    #[test]
    fn typed_permission_receipt_refuses_supervisor_bound_ask() {
        use bamboo_agent_core::tools::ExecutingSupervisorObservation;
        use bamboo_tools::permission::{
            ConfigPermissionChecker, PermissionConfig, PermissionDecisionKind,
            PermissionReasonCode, PermissionRequest, PermissionType, RiskLevel,
        };

        // 宿主 Supervisor 会话上的待答问题带 executing-supervisor 权威
        // 记录：聊天文本应答不得代铸 typed 回执（须走正式决策端点）。
        let config = Arc::new(PermissionConfig::new());
        config.register_pending_request(PermissionRequest {
            request_id: "call-1".to_string(),
            request_generation: "generation-7".to_string(),
            session_id: "session".to_string(),
            workspace_path: None,
            tool_name: "Bash".to_string(),
            permission_type: PermissionType::ExecuteCommand,
            resource: "current-command".to_string(),
            operation_summary: "run a command".to_string(),
            risk_level: RiskLevel::Medium,
            reason_code: PermissionReasonCode::ConfiguredAlwaysAsk,
            effective_mode: bamboo_domain::PermissionMode::Default,
            bypass_requested: false,
            auto_approve_requested: false,
            policy_revision: 0,
            matched_rule: None,
            allowed_decisions: vec![
                PermissionDecisionKind::AllowOnce,
                PermissionDecisionKind::DenyOnce,
            ],
            suggested_matchers: Vec::new(),
        });
        let checker: Arc<dyn bamboo_tools::permission::PermissionChecker> =
            Arc::new(ConfigPermissionChecker::new(config));

        let pending = bamboo_agent_core::PendingQuestion {
            tool_call_id: "call-1".to_string(),
            tool_name: "Bash".to_string(),
            question: "Permission required".to_string(),
            options: vec!["Approve".to_string(), "Deny".to_string()],
            allow_custom: false,
            source: bamboo_agent_core::PendingQuestionSource::PauseTool,
        };
        let mut session = Session::new("session", "model");
        let mut result = Message::tool_result("call-1", "{}".to_string());
        result.metadata = Some(serde_json::json!({
            ExecutingSupervisorObservation::PERMISSION_REPLAY_METADATA_KEY: {
                "session_id": "supervisor-default",
                "version": 1
            }
        }));
        session.add_message(result);

        assert!(typed_permission_receipt(
            &checker, "session", &session, &pending, "Approve", false
        )
        .expect("no error")
        .is_none());
    }

    fn append_permission_round(
        session: &mut Session,
        call_id: &str,
        generation: &str,
        arguments: &str,
        result_id: &str,
    ) {
        session.add_message(Message::assistant(
            "",
            Some(vec![ToolCall {
                id: call_id.to_string(),
                tool_type: "function".to_string(),
                function: FunctionCall {
                    name: "Write".to_string(),
                    arguments: arguments.to_string(),
                },
            }]),
        ));
        let mut result = Message::tool_result(
            call_id,
            serde_json::json!({
                "status": "awaiting_permission_approval",
                "permission_request": { "request_generation": generation }
            })
            .to_string(),
        );
        result.id = result_id.to_string();
        session.add_message(result);
    }

    fn ask(options: Vec<&str>, allow_custom: bool) -> ParkedAsk {
        ParkedAsk {
            nonce: "abc12345".to_string(),
            session_id: "sess-1".to_string(),
            tool_call_id: "call-1".to_string(),
            tool_name: "conclusion_with_options".to_string(),
            question: "Approve?".to_string(),
            options: options.into_iter().map(str::to_string).collect(),
            allow_custom,
        }
    }

    #[test]
    fn connect_replay_targets_current_generation_when_provider_reuses_id() {
        let mut session = Session::new("session", "model");
        append_permission_round(
            &mut session,
            "reused",
            "generation-old",
            r#"{"content":"old"}"#,
            "result-old",
        );
        append_permission_round(
            &mut session,
            "reused",
            "generation-current",
            r#"{"content":"current"}"#,
            "result-current",
        );

        let target = find_pending_tool_call(&session, "reused", Some("generation-current"))
            .expect("current generation target");
        assert_eq!(
            target.tool_call().function.arguments,
            r#"{"content":"current"}"#
        );
        assert!(apply_tool_result(
            &mut session,
            &target,
            "executed current".to_string(),
            true,
        ));
        assert!(session.messages[1].content.contains("generation-old"));
        assert_eq!(session.messages[3].content, "executed current");
    }

    #[test]
    fn new_nonce_is_short_and_hex_like() {
        let nonce = new_nonce();
        assert!(!nonce.is_empty());
        assert!(nonce.len() <= 16);
        assert!(nonce.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn match_text_answer_numeric_index_selects_option() {
        let pending = ask(vec!["Approve", "Deny"], false);
        assert_eq!(
            match_text_answer(&pending, "1"),
            Some("Approve".to_string())
        );
        assert_eq!(match_text_answer(&pending, "2"), Some("Deny".to_string()));
        assert_eq!(match_text_answer(&pending, "3"), None);
    }

    #[test]
    fn match_text_answer_exact_text_is_case_insensitive() {
        let pending = ask(vec!["Approve", "Deny"], false);
        assert_eq!(
            match_text_answer(&pending, "approve"),
            Some("Approve".to_string())
        );
    }

    #[test]
    fn match_text_answer_binary_keyword_mapping() {
        let pending = ask(vec!["Approve", "Deny"], false);
        assert_eq!(
            match_text_answer(&pending, "允许"),
            Some("Approve".to_string())
        );
        assert_eq!(
            match_text_answer(&pending, "yes"),
            Some("Approve".to_string())
        );
        assert_eq!(
            match_text_answer(&pending, "deny"),
            Some("Deny".to_string())
        );
        assert_eq!(match_text_answer(&pending, "no"), Some("Deny".to_string()));
    }

    /// Ordering guarantee documented on [`NEGATIVE_KEYWORDS`]: an option
    /// literally titled "Stay" — even as the POSITIVE first choice — resolves
    /// via exact option-text matching BEFORE the keyword fallback, so the
    /// "stay"-is-negative heuristic can never misroute it.
    #[test]
    fn match_text_answer_exact_option_named_stay_beats_negative_keyword_fallback() {
        let pending = ask(vec!["Stay", "Leave"], false);
        assert_eq!(
            match_text_answer(&pending, "stay"),
            Some("Stay".to_string())
        );
        // And the fallback still works as intended for plan-mode phrasing,
        // where "stay" appears INSIDE the negative option's text.
        let plan_pending = ask(vec!["Approve", "Stay in plan mode"], false);
        assert_eq!(
            match_text_answer(&plan_pending, "stay"),
            Some("Stay in plan mode".to_string())
        );
    }

    #[test]
    fn match_text_answer_closed_ask_non_matching_text_falls_through() {
        let pending = ask(vec!["Approve", "Deny"], false);
        assert_eq!(match_text_answer(&pending, "banana"), None);
    }

    #[test]
    fn match_text_answer_open_question_accepts_any_free_text() {
        let pending = ask(vec!["OK", "Need changes"], true);
        assert_eq!(
            match_text_answer(&pending, "please add tests too"),
            Some("please add tests too".to_string())
        );
    }

    #[test]
    fn match_text_answer_empty_text_never_matches() {
        let pending = ask(vec!["OK", "Need changes"], true);
        assert_eq!(match_text_answer(&pending, "   "), None);
    }

    #[test]
    fn match_callback_data_requires_the_exact_nonce() {
        let pending = ask(vec!["Approve", "Deny"], false);
        assert_eq!(
            match_callback_data(&pending, "abc12345:0"),
            Some("Approve".to_string())
        );
        assert_eq!(match_callback_data(&pending, "stale-nonce:0"), None);
    }

    #[test]
    fn match_callback_data_rejects_out_of_range_index() {
        let pending = ask(vec!["Approve", "Deny"], false);
        assert_eq!(match_callback_data(&pending, "abc12345:9"), None);
    }

    #[test]
    fn match_callback_data_rejects_malformed_data() {
        let pending = ask(vec!["Approve", "Deny"], false);
        assert_eq!(match_callback_data(&pending, "not-a-valid-shape"), None);
        assert_eq!(match_callback_data(&pending, "abc12345:not-a-number"), None);
    }

    #[test]
    fn format_ask_text_numbers_every_option() {
        let mut pending = ask(vec!["Approve", "Deny"], false);
        pending.question = "**Permission required**\n\nThe `Bash` tool needs approval to Execute shell commands on:\n\n`cargo build`".to_string();
        let text = format_ask_text(&pending);
        // 权限提示（固定 Approve/Deny 两项）问题正文汉化，展示中文标签，
        // 并提示 "都允许" 的会话级授权用法。
        assert!(text.contains("需要权限确认：执行命令"));
        assert!(text.contains("cargo build"));
        assert!(text.contains("Bash"));
        assert!(text.contains("1. 允许"));
        assert!(text.contains("2. 拒绝"));
        assert!(text.contains("都允许"));
        assert!(!text.contains("reply with your own answer"));
        assert!(!text.contains("Permission required"));
    }

    #[test]
    fn translate_permission_question_handles_unknown_shapes() {
        // 非权限模板原样返回；未知动作退化为通用中文格式。
        assert_eq!(
            translate_permission_question("随便一个问题"),
            "随便一个问题"
        );
        let unknown = translate_permission_question(
            "**Permission required**\n\nThe `X` tool needs approval to Do something exotic on:\n\n`res`",
        );
        assert!(unknown.contains("需要权限确认"));
        assert!(unknown.contains("Do something exotic"));
    }

    #[test]
    fn match_text_answer_supports_skip_and_cancel_intents() {
        // 中文回复命中英文选项，英文回复命中中文选项。
        let english = ask(vec!["Proceed", "Skip this file", "Cancel"], false);
        assert_eq!(
            match_text_answer(&english, "跳过").as_deref(),
            Some("Skip this file")
        );
        assert_eq!(
            match_text_answer(&english, "cancel").as_deref(),
            Some("Cancel")
        );
        let chinese = ask(vec!["继续执行", "跳过这个", "取消任务"], false);
        assert_eq!(
            match_text_answer(&chinese, "skip").as_deref(),
            Some("跳过这个")
        );
        // 通用子串层：短回复唯一命中。
        assert_eq!(
            match_text_answer(&chinese, "继续").as_deref(),
            Some("继续执行")
        );
        // 歧义（"取消" 命中 "取消任务" 唯一？——"取消任务" 含 "取消"，唯一命中 ✓）。
        assert_eq!(
            match_text_answer(&chinese, "取消").as_deref(),
            Some("取消任务")
        );
    }

    #[test]
    fn format_ask_text_keeps_custom_options_verbatim() {
        let pending = ask(vec!["OK", "Need changes"], false);
        let text = format_ask_text(&pending);
        assert!(text.contains("1. OK"));
        assert!(text.contains("2. Need changes"));
        assert!(text.contains("回复数字即可选择"));
    }

    #[test]
    fn format_ask_text_open_question_mentions_free_text() {
        let pending = ask(vec!["OK", "Need changes"], true);
        assert!(format_ask_text(&pending).contains("reply with your own answer"));
    }
}
