//! The `Platform` trait + shared inbound/outbound message types.
//!
//! Per epic #447: cc-connect's proven 5-method core, with capabilities
//! EXPLICIT (not type-asserted) so the bridge/render layer never guesses what
//! an adapter can do. `ReplyCtx` is platform-opaque (`serde_json::Value`),
//! carried on every [`InboundMessage`] and handed back to
//! `Platform::reply`/`Platform::edit` unmodified — mirrors cc-connect's
//! `replyCtx any` pattern.

use tokio::sync::mpsc;

/// What a platform adapter supports. MVP adapters (Telegram, phase 1) advertise
/// everything `false` except plain text — buttons/streaming-edit are phase 2.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Inline approval/action buttons on a message.
    pub buttons: bool,
    /// In-place message editing (`Platform::edit`) — used for streaming
    /// edit-in-place in a later phase.
    pub edit_message: bool,
    /// Sending image attachments.
    pub images: bool,
    /// Sending file attachments.
    pub files: bool,
    /// 适配器支持出站附件投递：解析回复文本中的 `[SEND_FILE: 绝对路径]`
    /// 标记行，把对应本地文件上传并发送给用户（标记行本身不展示）。
    /// bridge 会在此能力开启时向新会话注入一次约定说明；不支持的平台
    /// 原样忽略标记文本。目前仅微信（iLink）适配器开启。
    pub attachments: bool,
    /// 是否向聊天渠道逐条推送工具执行进度行（`⚙ 工具名: 参数摘要`）。
    /// 关闭后用户只看到最终回复、待答问题和错误提示——适合 IM 场景
    /// （微信）：中间过程的命令行噪音会淹没正文。其余平台保持开启。
    pub tool_progress: bool,
}

/// Platform-opaque context handed back unmodified to `Platform::reply`/`edit`.
/// Concrete shape is decided by each adapter (e.g. Telegram stores `chat_id`).
#[derive(Debug, Clone, PartialEq)]
pub struct ReplyCtx(pub serde_json::Value);

/// A reference to a previously-sent message, for `Platform::edit`
/// (capability-gated on [`Capabilities::edit_message`]).
#[derive(Debug, Clone, PartialEq)]
pub struct MessageRef(pub serde_json::Value);

/// A message received from a platform, normalized to the shape the bridge
/// understands. Fields beyond `text`/`reply_ctx` exist to let the bridge
/// enforce security policy (allow-list, dedup) generically across every
/// platform adapter, without knowing platform-specific wire formats.
#[derive(Debug, Clone)]
pub struct InboundMessage {
    /// Platform name (matches `Platform::name`), e.g. `"telegram"`.
    pub platform: String,
    /// Platform-scoped chat identifier.
    pub chat_id: String,
    /// Platform-scoped sender identifier (checked against `allow_from`).
    pub user_id: String,
    /// Platform-scoped, per-platform-unique message id used for dedup (e.g.
    /// Telegram's `update_id`, stringified).
    pub message_id: String,
    /// When the platform says the message was sent — used to drop stale
    /// backlog delivered right after a restart (older than process start).
    pub sent_at: chrono::DateTime<chrono::Utc>,
    /// Message text.
    pub text: String,
    /// Opaque context to hand back to `Platform::reply`/`edit`.
    pub reply_ctx: ReplyCtx,
}

/// One inline approval/action button (phase 2, issue #458). `callback_data`
/// is echoed back verbatim on press — the ONLY thing it may carry is a short
/// ask-nonce + option selector (`"{nonce}:{option_index}"`, see
/// `connect::approvals`), NEVER raw user text or anything else sensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub label: String,
    pub callback_data: String,
}

impl Button {
    pub fn new(label: impl Into<String>, callback_data: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            callback_data: callback_data.into(),
        }
    }
}

/// A message to send to a platform.
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub text: String,
    /// Inline keyboard rows (capability-gated on [`Capabilities::buttons`]).
    /// Adapters that don't advertise `buttons` ignore this field.
    pub buttons: Option<Vec<Vec<Button>>>,
}

impl OutboundMessage {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            buttons: None,
        }
    }

    pub fn with_buttons(mut self, buttons: Vec<Vec<Button>>) -> Self {
        self.buttons = Some(buttons);
        self
    }
}

/// An inline-button press received from a platform (capability-gated on
/// [`Capabilities::buttons`]) — the counterpart to a text [`InboundMessage`].
#[derive(Debug, Clone)]
pub struct CallbackQuery {
    /// Platform name (matches `Platform::name`), e.g. `"telegram"`.
    pub platform: String,
    /// Platform-scoped chat identifier.
    pub chat_id: String,
    /// Platform-scoped sender identifier (checked against `allow_from`,
    /// exactly like [`InboundMessage::user_id`]).
    pub user_id: String,
    /// Platform-scoped callback-query identifier. Every callback query MUST
    /// be acknowledged via `Platform::answer_callback` exactly once — success
    /// or not (stale/forged data still gets acked, just silently dropped).
    pub callback_query_id: String,
    /// The pressed button's `callback_data`, verbatim.
    pub data: String,
    /// Opaque context to hand back to `Platform::reply`/`edit`.
    pub reply_ctx: ReplyCtx,
}

/// Everything a platform can push onto its inbound channel: a text message or
/// a button press. Kept as one enum (rather than two channels) so a platform
/// whose transport interleaves both in a single feed (Telegram's
/// `getUpdates`) preserves delivery order end to end.
#[derive(Debug, Clone)]
pub enum Inbound {
    Message(InboundMessage),
    Callback(CallbackQuery),
}

/// Error returned by a `Platform` adapter operation.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("{0}")]
    Other(String),
}

impl PlatformError {
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}

pub type PlatformResult<T> = std::result::Result<T, PlatformError>;

/// An IM-platform adapter. cc-connect's proven 5-method core (`start`,
/// `reply`, `edit`, `stop`, plus `capabilities`/`name`) — see epic #447.
#[async_trait::async_trait]
pub trait Platform: Send + Sync {
    /// Short identifier, e.g. `"telegram"`. Matches [`InboundMessage::platform`].
    fn name(&self) -> &str;

    /// What this adapter supports.
    fn capabilities(&self) -> Capabilities;

    /// Start receiving inbound events (messages and/or button presses),
    /// sending each onto `inbound`. Runs for the adapter's lifetime (a
    /// long-poll loop, a WS connection, …); returns only on an unrecoverable
    /// error or `stop()`.
    async fn start(&self, inbound: mpsc::Sender<Inbound>) -> PlatformResult<()>;

    /// Send a message in reply to `ctx`.
    async fn reply(&self, ctx: &ReplyCtx, msg: OutboundMessage) -> PlatformResult<MessageRef>;

    /// Edit a previously-sent message in place. Capability-gated on
    /// [`Capabilities::edit_message`]; adapters that don't support it may
    /// return an error — callers must check the capability first.
    async fn edit(&self, msg_ref: &MessageRef, new: OutboundMessage) -> PlatformResult<()>;

    /// Acknowledge a callback query (capability-gated on
    /// [`Capabilities::buttons`] — adapters without button support never
    /// receive one to acknowledge, so the default no-op is correct for them).
    /// Telegram requires exactly one ack per callback query, success or not;
    /// `text`, when `Some`, is shown as a brief toast (e.g. an "expired"
    /// notice for a stale or forged nonce).
    async fn answer_callback(
        &self,
        _callback_query_id: &str,
        _text: Option<&str>,
    ) -> PlatformResult<()> {
        Ok(())
    }

    /// "正在输入中"指示（picoclaw/openclaw 同款体验）：机器人在后台思考
    /// 期间让聊天窗口显示 typing 状态。bridge 在一次运行期间周期性调用，
    /// 发出首条回复后停止。默认 no-op——只有支持该指示的适配器（微信
    /// iLink `sendtyping`）实现它；失败只记日志，绝不影响运行本身。
    async fn send_typing(&self, _ctx: &ReplyCtx) -> PlatformResult<()> {
        Ok(())
    }

    /// 主动停止"正在输入中"指示（如微信 sendtyping 的 status=2 cancel）。
    /// 默认 no-op；不支持或未开始过指示的平台直接成功。
    async fn stop_typing(&self, _ctx: &ReplyCtx) -> PlatformResult<()> {
        Ok(())
    }

    /// "信任首个发信人"闩锁（v0.0.4）：配置开启且尚未记录首信人时为 true。
    /// bridge 据此放行第一条入站消息并记录其发送者，而不是拒绝它。
    fn wants_first_sender_trust(&self) -> bool {
        false
    }

    /// 已记录的首信人 user id（平台自有持久化，重启不丢，同游标先例）。
    fn trusted_first_sender(&self) -> Option<String> {
        None
    }

    /// 记录首信人（一次性）。至多被调用一次；实现方负责落盘。
    fn record_trusted_first_sender(&self, _user_id: &str) {}

    /// Stop the adapter (best-effort; graceful shutdown).
    async fn stop(&self) -> PlatformResult<()>;
}
