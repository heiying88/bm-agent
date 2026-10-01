# `ask_agent` —— 面向子代理的持久 Mailbox 请求/应答

> 状态：设计 + 实现计划。基于 Change A（仅 actor 的子代理）。
> 已由用户批准的决策：传输 = **文件 Mailbox**（持久化，而非 WS）；
> 呈现面 = **`SubAgent` 工具上的 `action="ask"`**；答案来源 = **两种模式**
> （`query` 摘要/提取 + `steer` 注入/改向）；非活跃 = **auto-activate**。

## 1. 为什么用 Mailbox，它又放在哪里

`Mailbox` 类型（`crates/infra/bamboo-subagent/src/mailbox.rs`，maildir `new/cur/corrupt`，`InboxKind::{Task,Ask,Handoff,Reply}`）已完整建成但处于**休眠**——只有 `Registry` 和测试碰它；活跃 actor 路径从不排空邮箱。父方（服务端）与 worker（`bamboo subagent-worker`）之间活跃的跨进程协调点是共享的 **`fabric_dir`**（`ProvisionSpec.fabric_dir`，默认 `$TMP/bamboo-subagents`），worker 在那里自行注册发现记录（`Fabric`）。

因此我们把 ask/reply 邮箱根植在共享 fabric 之下：

```
<fabric_dir>/mailboxes/<session_id>/{new,cur,corrupt}/    # one mailbox per session
```

- **Ask** → 投递到 `<fabric>/mailboxes/<child_id>/`（目标 child 的收件箱）。
- **Reply** → 投递到 `<fabric>/mailboxes/<parent_id>/`（发起询问的父方收件箱）。

两侧都构造 `Mailbox::at(<fabric>/mailboxes/<id>)`——不需要 `SubagentStore`。

## 2. 数据模型（mailbox.rs）

`InboxMessage` 增加一个关联 id，使 Reply 能匹配到它的 Ask：

```rust
pub struct InboxMessage {
    pub id: MsgId,
    pub from: AgentRef,                 // 发送方（Ask 为父方，Reply 为 child）
    pub kind: InboxKind,                // Ask | Reply（其余已存在）
    pub body: serde_json::Value,        // Ask：{question, mode}；Reply：{answer}
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<MsgId>,  // 新增：Reply.correlation_id == Ask.id
}
```

`AgentRef.session_id` 已经携带了我们需要的寻址信息（父方还是 child 的 id）。Body 辅助类型：`AskBody { question: String, mode: AskMode }`、`ReplyBody { answer: String }`、`enum AskMode { Query, Steer }`（serde snake_case，默认 `Query`）。

## 3. 供给（provision.rs）

worker 必须知道 (a) fabric（已经有了）、(b) 自己的 session id、(c) 父方的 session id（用于给应答寻址）。`ChildIdentity` 已携带代理/session 身份；若尚不可推导，则在 `ProvisionSpec` 上加 `parent_session_id`（`serde(default)`，前向兼容）。worker 由此推导：

- 自己的收件箱 = `Mailbox::at(<fabric>/mailboxes/<own_session_id>)`
- 应答收件箱 = `Mailbox::at(<fabric>/mailboxes/<parent_session_id>)`

## 4. Worker 侧（subagent_worker.rs）——在已知节点排空（不做忙轮询）

新增 `drain_asks(&own_mailbox, &reply_mailbox, &agent, &session)`：

1. `own_mailbox.drain()` → 对每条 `Ask` `InboxMessage`（经 `AdmittedSet` 去重）：
   - `Query` 模式：克隆当前 session，在克隆上跑一次简短的 `agent.execute(question)`，提取最终的 assistant 消息（复用 `subagent_worker.rs:359-366` 处的运行结果提取）。活跃任务不受影响。
   - `Steer` 模式：把 `question` 作为一条 user 消息注入活跃 session 并运行；得到的最终 assistant 消息即答案（这就是"改写目标"的路径）。
   - 把 `Reply { answer, correlation_id = ask.id, from = child }` 投递到 `reply_mailbox`。
   - `ack` 该 Ask。

在以下时点调用 `drain_asks`：worker **启动时**（第一次 `Run` 之前/前后；接住 auto-activate 期间排队的 ask）、每个**回合边界**（`SteerInbox` 本来就在这里排空）、以及新的 **`ParentFrame::DrainAsks`** WS 轻推（活跃时促发排空）。每个 worker 串行处理 ask（一次一条），以约束 LLM 负载。

## 5. 父方

- **引擎**：`ChildSessionPort::ask_child(child_id, question, mode, timeout) -> Result<String>` + `send_message_to_child_action` 附近的 `ask_child_action`（`child_session/actions.rs:337`）：`load_child_for_parent` 作用域守卫 → 若给出常驻名则解析 → 把 Ask 投递到 child 的 fabric 邮箱 → 若 `!is_live` 则 **auto-activate**（经 actor runner 入队/拉起），否则发送 `ParentFrame::DrainAsks` 轻推 → 轮询父方 reply 邮箱中 `correlation_id == ask.id` 的 `Reply`，以 `timeout` 为界 → `ack` + 返回 `answer`。
- **服务端适配器**（`child_session_adapter.rs`）：基于 fabric 邮箱 + `external_agents::live`（轻推 / is_live）+ 拉起调度器（auto-activate）实现 `ask_child`。

## 6. 工具面（sub_agent.rs）

新增 `SubAgentArgs::Ask { child_session_id: Option<String>, resident_name: Option<String>, question: String, mode: Option<AskMode>, timeout_secs: Option<u64> }`。分发逻辑在 `:849` 附近：解析目标（裸 id，或按 `:539-557` 走 `find_resident_child`），调用 `ask_child_action`，返回 `tool_result({ from, answer, mode, status: "answered" })`（同步——不经过 `register_parent_wait_for_child`）。在工具描述中写明 `action="ask"`（提及 `ask_agent` 别名）。`SERVER_TOOL_NAMES` 不变。

## 7. 实现分层（每层先通过编译 + 测试，再进入下一层）

1. **数据/协议**：`InboxMessage.correlation_id`、`AskBody`/`ReplyBody`/`AskMode`、`ParentFrame::DrainAsks`。单元测试（round-trip）。
2. **Worker 排空**：`drain_asks` + 调用点 + `DrainAsks` 处理。Echo 执行器测试。
3. **引擎端口 + action**：`ask_child` trait 方法、`ask_child_action`。Fake-port 测试。
4. **服务端适配器**：fabric 邮箱投递/轮询 + auto-activate + 轻推。
5. **工具 action** + 描述 + 常驻名解析。`sub_agent_tests` 覆盖。
6. 端到端测试（worktree 集成）：向一个活跃的 echo child 发起 ask，断言应答。

## 8. 待定/可再确认项

- 模式默认 = `query`。Steer 模式会改动 child 的活跃对话。
- Auto-activate 会在 worker 空闲/死亡时拉起它；从未创建过的 child 仍会报错（不是调用者的 child）。
- 并发：v1 中每个 worker 的 ask 串行处理。
