# bamboo-broker 核心路由——深度分析

范围：`crates/app/bamboo-broker/src/{proto,core,lib,error}.rs`，辅以 `bamboo-subagent/src/mailbox.rs`（broker 所依赖的底层 maildir 存储）作为佐证。除非带 `mailbox.rs:` 前缀，所有行号均为 1 起始，且指本 crate 内的文件。

---

## 1. `error.rs`——错误面

一个 21 行的文件，包含两项：

| 条目 | 位置 | 说明 |
|---|---|---|
| `enum BrokerError` | `error.rs:5-19` | `#[derive(Debug)]` + `thiserror::Error`。四个变体。 |
| `type BrokerResult<T> = Result<T, BrokerError>` | `error.rs:21` | 全 crate 通用的 Result 别名。 |

变体：

| 变体 | 行 | `#[error(...)]` | `#[from]`？ | 生产方 |
|---|---|---|---|---|
| `Store(StoreError)` | `error.rs:9` | `"store: {0}"` | **是**——`bamboo_subagent::StoreError` | 任意 `Mailbox` 操作（`deliver`/`drain`/`recover`/`ack`）经由 `?` 产生。参见 `core.rs:47`、`core.rs:68`、`core.rs:71`、`core.rs:85`、`core.rs:107`。 |
| `Auth(String)` | `error.rs:11` | `"auth: {0}"` | 否 | WS 认证层（token 校验）。完全不在 `core.rs` 内产生。 |
| `Protocol(String)` | `error.rs:14` | `"protocol: {0}"` | 否 | 乱序帧（例如在 `Hello` 之前发请求）。传输层关注点。 |
| `Transport(String)` | `error.rs:17` | `"transport: {0}"` | 否 | WebSocket / IO 故障。 |

**观察**
- `Store` 是枚举中唯一的 `#[from]`，因此对 `StoreError` 使用 `?` 是本 crate 中唯一的隐式转换。`Auth`/`Protocol`/`Transport` 必须显式构造，如 `BrokerError::Auth(..)`。
- **没有 `Serialization`/`Json` 变体。**`proto.rs:44` 和 `proto.rs:53` 使用 `serde_json::to_string(...).expect(...)`——序列化失败时直接 panic，而不是变成 `BrokerError`。之所以说得过去，仅因为输入都是自序列化的 DTO；畸形的 `InboxMessage.body`（任意 `serde_json::Value`）不可能重新序列化失败，所以这个 `expect` 实际上是安全的，但值得作为一个 panic 面记录在案。
- `from_text`（`proto.rs:46`、`proto.rs:55`）返回 `serde_json::Result`，而**不是** `BrokerResult`。WS 层必须手工把 `serde_json::Error` 提升为 `BrokerError::Protocol(..)`；没有 `#[from] serde_json::Error`。这是一个轻微的人机工程缺口。
- `BrokerError` **不是** `Clone`。`Auth`/`Protocol`/`Transport` 装载 `String`；`Store(#[from] StoreError)` 则取决于 `StoreError` 本身（多半同样不是 `Clone`）。这对 `?` 传播没有影响，但意味着测试无法直接对错误 `assert_eq!`。

---

## 2. `proto.rs`——线上协议

### 2.1 类型复用哲学
`proto.rs:1-5` 说得很明白：broker 是 `bamboo_subagent` 消息类型的**传输通道**，不对它们做再解释。唯一的 `use`（`proto.rs:7`）是：
```rust
use bamboo_subagent::{AgentRef, InboxMessage, MsgId};
```
因此 `InboxMessage`（规范的收件箱负载）、`MsgId`（其 id 类型）和 `AgentRef`（session 身份）被原样重新导出到线上格式中。消息外面**没有 broker 私有的信封**——`BrokerFrame::Message { message: InboxMessage }`（`proto.rs:37`）直接传输接收方 `Mailbox` 本会存储的那个结构体。含义：对 `InboxMessage` 的任何 schema 变更对 broker 都是对线上协议的破坏性变更，且没有任何版本字段可以用来把关。

### 2.2 `ClientFrame`（客户端 → broker），`proto.rs:10-25`
`proto.rs:12` 处的 `#[serde(tag = "kind", rename_all = "snake_case")]`——内部打标的 JSON 枚举，标签键为 `"kind"`，变体名转为小写蛇形。

| 变体 | 行 | 字段 | 语义 |
|---|---|---|---|
| `Hello` | `proto.rs:16` | `agent: AgentRef`, `token: String` | 首帧。把连接绑定到 mailbox 键 `agent.session_id`。`token` 是认证凭据。 |
| `Deliver` | `proto.rs:18` | `to: String`, `message: InboxMessage` | 持久入队到 `to` 的 mailbox。`to` 是裸 session id（不含 `AgentRef`），因此只按 session id 路由。 |
| `Subscribe` | `proto.rs:21` | （单元变体） | 开始推送调用方**自己的** mailbox。 |
| `Ack` | `proto.rs:24` | `id: MsgId` | 确认（删除）一条已推送的消息。至少一次重投递的关键支点。 |

`proto.rs:104-106` 的标签稳定性测试把单元变体的 `kind == "subscribe"` 钉死——正是这条契约防止未来某次 `rename_all` 变更悄悄破坏线上协议。

### 2.3 `BrokerFrame`（broker → 客户端），`proto.rs:27-40`
采用相同的 serde 标签风格（`proto.rs:29`）。

| 变体 | 行 | 字段 | 语义 |
|---|---|---|---|
| `Welcome` | `proto.rs:32` | （单元变体） | 握手被接受。 |
| `Error` | `proto.rs:35` | `reason: String` | 拒绝；认证错误后 broker 关闭连接。 |
| `Message` | `proto.rs:37` | `message: InboxMessage` | 一条推送的 mailbox 消息。 |
| `Delivered` | `proto.rs:39` | `id: MsgId` | 对已处理 `Deliver`（已持久入队）的回执。 |

**值得注意的不对称：**`ClientFrame::Deliver` 在 `BrokerCore` API 中按引用接收消息（`core.rs:46`：`msg: &InboxMessage`），但帧本身拥有它（`proto.rs:18`）。WS 层在分发时必须把帧中拥有的字段移动到这个借用里。

### 2.4 `to_text` / `from_text`，`proto.rs:42-58`
```rust
pub fn to_text(&self) -> String { serde_json::to_string(self).expect("… serializes") }
pub fn from_text(s: &str) -> serde_json::Result<Self> { serde_json::from_str(s) }
```
两个枚举上对称的辅助函数。两点观察：
1. `to_text` 在序列化失败时**panic**（见 §1）。`proto.rs:84-107` 和 `proto.rs:109-122` 的往返测试证明了 `from_text(to_text(x))` 之后相等。
2. `from_text` 返回 `serde_json::Result`，不是 `BrokerResult`。`BrokerError` 上没有 `#[from] serde_json::Error`，所以 WS 层必须手工把它映射成 `BrokerError::Protocol(..)`。这是一个真实的摩擦点——`serve`/`server` 中 `from_text` 的每个调用点都要付这笔税。

### 2.5 `Hello` 中的 `token`，`proto.rs:16`
认证 token 是 `Hello` 上的一个普通 `String` 字段。`proto.rs` 内部没有任何校验——该责任完全推给了 WS 层（它会发出 `BrokerFrame::Error { reason }` 并关闭连接）。`core.rs` **没有**认证概念；`BrokerCore::deliver`/`subscribe`/`ack` 无条件接受任何 session id。这是一个干净的分离（core 与传输无关、与认证无关），但也意味着安全边界完全位于调用 `BrokerCore` 的那一层。`token` 以明文放在 JSON 文本帧中发送；TLS 是 WS 层要解决的问题。

### 2.6 测试覆盖，`proto.rs:60-122`
`ask_msg()`（`proto.rs:66-82`）构造一个带 `InboxKind::Ask` + `AskBody { question, mode: AskMode::Query }` 的 `InboxMessage`。两个往返测试都遍历了所有变体。标签稳定性断言（`proto.rs:105-106`）是唯一的线上格式锚点；**没有**负面测试（畸形 JSON、未知 `kind` 标签、缺失字段）。目前未知 `kind` 会让 `from_str` 以 serde 错误失败——前向兼容（忽略未知标签）**没有**提供。

---

## 3. `core.rs`——路由引擎

### 3.1 状态，`core.rs:25-29`
```rust
pub struct BrokerCore {
    root: PathBuf,
    subscribers: Mutex<HashMap<String, mpsc::UnboundedSender<InboxMessage>>>,
}
```
- `root`——maildir 根目录。每个 session 的 mailbox 位于 `<root>/mailboxes/<session_id>`（`core.rs:40-42`）。对 `session_id` **没有任何清洗**：调用方在 `ClientFrame::Deliver`（`proto.rs:18`）中传入 `to: "../admin"` 就会在 mailbox 层形成路径穿越向量。只有当 WS/认证层在调用 `BrokerCore` 之前对 session id 做规范化或白名单过滤，这才是安全的。
- `subscribers`——`session_id → unbounded sender`。是 Tokio 的 `Mutex`（不是 `std`），因此允许跨 `.await` 持有（`subscribe` 正是这么做的——见 §3.4）。使用 `mpsc::UnboundedSender` 意味着推送永远不会阻塞生产者；如果订阅者的接收端很慢，消息会在 channel 中无限堆积（停滞的消费者带来内存压力）。

### 3.2 `mailbox()`，`core.rs:40-42`
```rust
fn mailbox(&self, session_id: &str) -> Mailbox {
    Mailbox::at(self.root.join("mailboxes").join(session_id))
}
```
每次调用都会**新建**一个 `Mailbox` 值。`Mailbox` 很廉价（只是一个 `PathBuf`，见 `mailbox.rs:125-127`），本身不持有任何状态。全部持久化状态都在磁盘上的 `new/`、`cur/`、`corrupt/` 子目录里（`mailbox.rs:129-137`）。这正是 broker 能跨重启无状态的原因——一切可恢复的东西都在磁盘上。

### 3.3 `deliver`，`core.rs:46-50`
```rust
pub async fn deliver(&self, to: &str, msg: &InboxMessage) -> BrokerResult<MsgId> {
    let id = self.mailbox(to).deliver(msg).await?;   // 持久化到 new/
    self.push_new(to).await?;                          // 已订阅则认领并推送
    Ok(id)
}
```
两个阶段：
1. **持久入队**——`Mailbox::deliver`（`mailbox.rs:151-159`）序列化为 JSON，将文件命名为 `<20 位纳秒>-<msgid>.json`（字典序 == 时间序，`mailbox.rs:153-155`），并 `atomic_write` 进 `new/`。返回 `MsgId`。这一行返回后，消息就能在崩溃中幸存。
2. **实时推送**——如果没有注册订阅者，`push_new(to)`（`core.rs:99-111`）是空操作；否则它会 `drain()` `new/` 并推送每条被认领的消息。关键在于，`push_new` 会排空 `new/` 中**所有**待处理消息，而不只是第 1 步投递的那条——因此对同一 session 的并发投递会合并成一次排空。

**顺序不变式：**由于 `Mailbox::deliver` 在磁盘上是同步的（原子写完成之后 `await` 才结束），且文件名前缀是 `timestamp_nanos_opt`，在同一个 broker 上按墙钟顺序投递的消息会按墙钟顺序被排空。这是尽力而为的全局顺序，不是因果顺序——两个发送方竞争时，时间戳的先后取决于 OS 调度 `atomic_write` 调用的顺序。

### 3.4 `subscribe`，`core.rs:55-75`——引擎的心脏
```rust
pub async fn subscribe(&self, session_id: &str)
    -> BrokerResult<mpsc::UnboundedReceiver<InboxMessage>>
{
    let (tx, rx) = mpsc::unbounded_channel();
    self.subscribers.lock().await.insert(session_id.to_string(), tx.clone());  // ← (A)

    let mb = self.mailbox(session_id);
    for d in mb.recover().await? { let _ = tx.send(d.msg); }                   // ← (B) cur/
    for d in mb.drain().await?  { let _ = tx.send(d.msg); }                    // ← (C) new/
    Ok(rx)
}
```
三个子阶段，依次是：

- **（A）注册**：把新的 sender 放进映射表。`HashMap::insert` 会**替换**同一 `session_id` 之前的 sender（`core.rs:54` 有文档说明：“同一 id 的先前订阅者会被替换”）。`tx.clone()` 严格来说没必要——`tx` 可以直接 move——但无害。映射表只在 `insert` 期间持锁；(B) 和 (C) 之前锁已释放。**这是核心的并发风险点，§5.1 中有分析。**
- **（B）恢复** `cur/`——`mailbox.rs:220-...` 读取 `cur/` 中已有的全部文件（上次连接/崩溃时已认领但未确认的）。解析失败的文件被移入 `corrupt/`（`mailbox.rs:231-232`）。这些会推送给新订阅者。
- **（C）排空** `new/`——`mailbox.rs:165-...` 原子地把 `new/<name>` 重命名为 `cur/<name>`（`mailbox.rs:170-175`），读取并推送。重命名失败（文件已被认领）会被跳过（`mailbox.rs:173-175`），因此并发排空是安全的。

`subscribe` 返回后，调用方持有 `rx`。sender 侧一直留在映射表中，直到 `unsubscribe` 或后续的 `subscribe` 将其替换。

### 3.5 `unsubscribe`，`core.rs:79-81`
```rust
pub async fn unsubscribe(&self, session_id: &str) {
    self.subscribers.lock().await.remove(session_id);
}
```
删除映射表项。`UnboundedSender` 在这里被 drop（如果没有剩余克隆——`subscribe` 中的那次克隆之后并无其他克隆），最终会使接收端的 `rx.recv()` 返回 `None`。**重要：**已经在 `cur/` 中的消息（已认领但未确认）**不会**在这里被删除——它们留在磁盘上，等下一次 `subscribe` 去 `recover`。这就是至少一次重投递机制，由 `core.rs:182-196` 的测试验证。

### 3.6 `ack`，`core.rs:84-87`
```rust
pub async fn ack(&self, session_id: &str, id: &MsgId) -> BrokerResult<()> {
    self.mailbox(session_id).ack(id).await?;
    Ok(())
}
```
委托给 `Mailbox::ack`（`mailbox.rs:199-...`）：扫描 `cur/`，找到文件名以 `-<msgid>.json` 结尾的文件并删除（`mailbox.rs:200, 209`）。如果 `cur/` 不存在，返回 `Ok(())`（`mailbox.rs:204-205`）——幂等。`ack` 之后，后续 `recover` 不会再推送该消息（`core.rs:167-180` 的测试）。

### 3.7 `push_new`，`core.rs:99-111`
```rust
async fn push_new(&self, session_id: &str) -> BrokerResult<()> {
    let tx = {
        let subs = self.subscribers.lock().await;
        match subs.get(session_id) {
            Some(tx) => tx.clone(),
            None => return Ok(()),
        }
    };                                                       // 锁在此处释放
    for d in self.mailbox(session_id).drain().await? {
        let _ = tx.send(d.msg);
    }
    Ok(())
}
```
- 只在**克隆 sender** 时持锁，随后在（缓慢的异步）`drain` 之前释放。这是刻意为之：缩短临界区长度，避免 `deliver` 在不同 session 之间被串行化。
- 如果接收端已被 drop（例如订阅者在克隆与发送之间断开），`let _ = tx.send(...)` 会静默丢弃这次发送。这是有意为之且安全的——消息已经在 `cur/` 中持久存在（drain 已重命名它），丢弃这次内存推送只是意味着下一次 `subscribe` 会 `recover` 它。
- docstring（`core.rs:94-98`）说得很精确：`push_new` **不会** `recover`。在线订阅者不会被自己尚未确认的消息重复轰炸；只有新的 `subscribe` 才会。

### 3.8 `is_subscribed`，`core.rs:90-92`
即 `subs.lock().await.contains_key(session_id)`。只在测试中使用（`core.rs:147`）。注意：返回 `true` **不**保证订阅者的接收端仍然存活——sender 还在映射表里时接收端可能已被 drop。这是一个存活提示，不是正确性原语。

### 3.9 测试，`core.rs:114-207`
五个测试，全部使用 `tempfile::TempDir` 和 `tokio::test`：
| 测试 | 行 | 验证内容 |
|---|---|---|
| `deliver_then_subscribe_drains_backlog` | `core.rs:142-152` | 未订阅期间投递会持久保存；稍后 subscribe 将其排空。 |
| `subscribe_then_deliver_pushes_live` | `core.rs:154-164` | 先订阅再投递会实时推送。 |
| `ack_removes_so_resubscribe_does_not_redeliver` | `core.rs:167-180` | 确认 + 取消订阅 + 重新订阅 ⇒ 不再重投。 |
| `unacked_message_redelivers_on_resubscribe` | `core.rs:182-196` | 不确认 + 取消订阅 + 重新订阅 ⇒ 通过 `recover` 重投。 |
| `deliver_to_unsubscribed_is_durable_and_isolated_per_session` | `core.rs:198-207` | 按 session 隔离：向 "a" 和 "b" 投递，"a" 的订阅者只能看到 "a"。 |

**覆盖缺口：**没有测试覆盖 (a) 两个订阅者在同一 `session_id` 上竞争，(b) `deliver` 与 `subscribe` 竞争，(c) `push_new` 在已死接收端上的静默丢弃，(d) 路径穿越的 `session_id`，(e) 多个发送方向同一 session 并发 `deliver`。这些缺口正是 §5 中风险所在之处。

---

## 4. `lib.rs`——公开 API 面

### 4.1 模块图，`lib.rs:19-28`
```rust
pub mod ask; pub mod client; pub mod core; pub mod deploy;
pub mod mcp;  pub mod proto; pub mod serve; pub mod server;
mod error;   // ← 私有；只有再导出的类型会对外暴露
```
八个公开模块，一个私有模块（`error`）。`error` 是私有的，但其类型被再导出（`lib.rs:40`），因此下游可以直接使用 `BrokerError`/`BrokerResult` 而无需看到该模块。

### 4.2 `ORCHESTRATOR_ID`，`lib.rs:30-32`
```rust
pub const ORCHESTRATOR_ID: &str = "bamboo-orchestrator";
```
中央 orchestrator 的知名 mailbox id。worker 把 MCP 代理请求发到这里；`serve_mcp_proxy` 在这里监听。注释强调“单一 MCP 宿主”——这是一个**固定的单例**，不是按实例分配的 id。同一磁盘根上的两个 broker 会在该 mailbox 上冲突；部署模型假设每个根只有一个 broker。

### 4.3 再导出，`lib.rs:34-45`
| 再导出项 | 行 | 来源 |
|---|---|---|
| `ask_agent, ask_over, request_over` | `lib.rs:34` | `crate::ask` |
| `BrokerClient` | `lib.rs:35` | `crate::client` |
| `BrokerCore` | `lib.rs:36` | `crate::core` |
| `AgentDeployment, DeployedAgent, Deployer, DockerDeployer, LocalProcessDeployer, SshDeployer` | `lib.rs:37-39` | `crate::deploy` |
| `BrokerError, BrokerResult` | `lib.rs:40` | `crate::error` |
| `serve_mcp_proxy, McpProxyExecutor, McpReply, McpRequest, ProxiedResult` | `lib.rs:41` | `crate::mcp` |
| `BrokerFrame, ClientFrame` | `lib.rs:42` | `crate::proto` |
| `serve_executor, serve_loop, serve_mailbox, serve_with, Handled` | `lib.rs:43` | `crate::serve` |
| `BrokerServer` | `lib.rs:44` | `crate::server` |
| `AgentRef` | `lib.rs:45` | `bamboo_subagent`（透传） |

`lib.rs:45` 再导出 `bamboo_subagent::AgentRef`，使使用方可以 `use bamboo_broker::AgentRef` 而不必直接依赖 `bamboo-subagent`。值得注意的是**只有 `AgentRef` 被再导出**，`InboxMessage` 和 `MsgId` 都没有——尽管 proto 帧内嵌了它们，调用方仍须深入 `bamboo_subagent` 获取。这是一个小的不一致。

crate 级文档（`lib.rs:1-17`）对拓扑讲得很清楚：**单一中央 broker、枢纽辐射型、纯消息总线、不做 actor 生成**。部署（placement）藏在 `bamboo_subagent::WorkerLauncher` 之后。正是这份设计契约支撑了 `core.rs` 中所有单 broker 假设。

---

## 5. 并发与正确性分析

### 5.1 竞争：同一 session 上 `deliver` 对 `subscribe` ⚠️ **消息重复，而非丢失**

跟踪 session `"s"` 上两个交错的任务：

| 步骤 | `deliver("s", m)` | `subscribe("s")` |
|---|---|---|
| 1 | `Mailbox::deliver(m)` → 文件出现在 `new/` | |
| 2 | | 把 `tx` 插入映射表（`core.rs:60-63`） |
| 3 | | `recover()` 读取 `cur/`——`m` 不在其中 |
| 4 | | `drain()` 把 `new/m` 重命名为 `cur/m`，将 `m` 推送给 `tx` |
| 5 | `push_new("s")`：克隆 `tx`，再次调用 `drain()` | |
| 6 | `drain()` 返回**空**（`m` 已在 `cur/`）→ 不推送 | |

结果：`m` 向订阅者投递了**一次**。maildir 重命名就是认领操作且是原子的，因此两个 `drain()` 不可能都认领 `m`（`mailbox.rs:172-175` 防护了重命名失败竞争）。**在这种交错下既无丢失也无重复。**

危险的交错是：

| 步骤 | `deliver("s", m)` | `subscribe("s")` |
|---|---|---|
| 1 | `Mailbox::deliver(m)` → `new/m` | |
| 2 | | `insert(tx)` |
| 3 | | `recover()`（空） |
| 4 | `push_new("s")`：克隆 `tx`，`drain()` 认领 `new/m` → `cur/m`，**将 `m` 推送给 `tx`** | |
| 5 | | `drain()` 返回空（已被认领） |
| 6 | | 返回 `rx` |

仍然只投递一次——`push_new` 赢得了第 4 步的重命名。再考虑：

| 步骤 | `deliver("s", m)` | `subscribe("s")` |
|---|---|---|
| 1 | `Mailbox::deliver(m)` → `new/m` | |
| 2 | | `insert(tx)` |
| 3 | | `recover()` 为空 |
| 4 | | `drain()` 认领 `new/m` → `cur/m`，将 `m` 推送给 `tx` |
| 5 | `push_new("s")`：克隆 `tx`，`drain()` 返回空，**不推送** | |

一次投递。maildir 的认领一次语义使同一 session 上的 `deliver`/`subscribe` **既不会丢失也不会重复**，*前提*是订阅者确实消费 `rx`。唯一的残余风险是第 4 步的 `tx.send` 被静默丢弃（接收端在第 2 步到第 4 步之间被 drop）：此时消息在 `cur/` 中未确认，会由**下一次** `subscribe` 的 `recover` 重新投递。仍然没有丢失。

**结论：**得益于 maildir 的认领一次语义，`deliver` 与 `subscribe` 的竞争是良性的。至少一次得以保持；精确一次并未承诺（文档也没有这样声称）。消费者必须按 `MsgId` 去重（`core.rs:13-14` 有说明）。

### 5.2 竞争：两个并发的 `subscribe("s")` ⚠️ **旧订阅者被静默孤立**

这是最隐蔽的风险。`core.rs:54` 写明“同一 id 的先前订阅者会被替换”，但其后果并不直观：

| 步骤 | 任务 A `subscribe("s")` | 任务 B `subscribe("s")` |
|---|---|---|
| 1 | `insert(tx_A)` | |
| 2 | | `insert(tx_B)`——**丢弃了映射表对 `tx_A` 的引用** |
| 3 | `recover()` 推送给 `tx_A` | |
| 4 | `drain()` 推送给 `tx_A` | |
| 5 | 返回 `rx_A` | （B 的 recover/drain 可能发现 `cur/`/`new/` 已被 A 排空） |

第 2 步之后，`tx_A` **只**被任务 A 的局部变量持有——映射表不再引用它。因此后续的 `deliver("s", …)` 调用经由 `push_new` 发往 `tx_B`，**永远不再发往 `tx_A`**。任务 A 的 `rx_A` 只收到第 3-4 步推送的内容，随后永远沉寂（channel 仍保持打开，因为任务 A 仍通过被 move 的 `tx` 持有 `tx_A` 的兄弟……其实不对——`tx_A` 是 A 中的局部变量；接收端 `rx_A` 只有在**所有** sender 都 drop 后才会返回 `None`。任务 A 持有 `tx` 直到 `subscribe` 返回，届时 `tx` 离开作用域——等等，`subscribe` 把 `tx` 克隆进映射表，局部 `tx` 在函数结束时被 drop。所以 A 返回后，`rx_A` 的 sender 数为零，`recv()` 返回 `None`。）

更准确地说：`subscribe` 返回后，`rx_A` **唯一**的 sender 是映射表中的那个（局部 `tx` 已被 move/克隆后 drop）。当 B 的 `insert` 覆盖它时，`rx_A` **不再有任何 sender** → `rx_A.recv()` 立即返回 `None`。任务 A 看到流结束，（如果写得好的话）会将其视为“已断开连接”。**这是有意为之的语义，但它是隐式的**——没有 `BrokerFrame::Error { reason: "superseded" }`。天真的消费者可能把静默的 `None` 理解为“mailbox 永远为空”，而不是“你已被替换”。

**被孤立订阅者的消息丢失场景：**如果任务 A 在第 3-4 步的 `recover()`/`drain()` 运行在 B 排空**之后**（B 完全先行的交错），那么 A 的 `drain()` 会发现 `new/` 为空，A 什么也拿不到——但 B 已经拿到了消息，系统层面并没有真正丢失。丢失只存在于“从 A 的视角”，而这是正确的，因为 A 已被取代。

**真正的问题：**如果 A 的 `drain()` 运行在 B 的 `insert` **之前**，A 会把消息认领进 `cur/` 并推送给 `rx_A`。此时 B 订阅；B 的 `recover()` 从 `cur/` 读到同样的消息（它们未确认）并推送给 `rx_B`。**A 和 B 都会看到相同的消息。**这是同一 session 的两个消费者之间的至少一次重投递——如果把模型理解为“session 的 mailbox 每次只有一个读取者，但读取者交接可能把在途消息投递两次”，这是预期行为。如果消费者按 `MsgId` 幂等，这**不是** bug；如果部署假设两个订阅者等于两个独立消费者，它就是 bug。

### 5.3 竞争：`unsubscribe` 对 `deliver` ⚠️ **良性（无丢失）**

| 步骤 | `deliver("s", m)` | `unsubscribe("s")` |
|---|---|---|
| 1 | `Mailbox::deliver(m)` → `new/m` | |
| 2 | | `remove("s")`——`tx` 从映射表中被 drop |
| 3 | `push_new("s")`：`get("s")` → `None` → 直接返回，**不推送** | |

`m` 留在 `new/` 中，持久存在。下一次 `subscribe("s")` 会 `drain()` 它。**没有丢失。**这就是投递中途断开连接时的设计行为。

### 5.4 竞争：`push_new` 克隆的 sender 对 `unsubscribe`——**静默丢弃，良性**

`push_new`（`core.rs:100-106`）在持锁期间克隆 `tx`，然后释放锁，再执行 `drain()` 和 `tx.send()`。如果 `unsubscribe` 在克隆与发送之间运行，发送目标是一个仍然存活的**克隆**（`push_new` 中的局部 `tx`），因此发送会成功进入一个接收端可能已经消失的 channel。`let _ = tx.send(...)` 丢弃了错误。消息在 `cur/` 中持久存在（drain 已重命名它），下一次 `subscribe` 会重新取回它。良性。

### 5.5 订阅者映射表的并发模型——**整表一把互斥锁，无按 session 的锁**

恰好有**一把** `Mutex<HashMap<..>>`（`core.rs:28`）守护**所有** session。后果：
- **优点：**`insert`/`remove`/`get` 的原子性不言而喻——互斥锁把它们串行化。
- **缺点：**向 session “x” 的缓慢 `deliver` **不会**阻塞向 “y” 的 `deliver`，因为 `push_new` 会在缓慢的 `drain()` 之前释放锁（`core.rs:100-106`）。这是好事。
- **缺点：**`subscribe` 在其（缓慢的）`recover`/`drain`（`core.rs:65-73`）期间不持锁，但它**已经修改了映射表**（`core.rs:60-63`）。因此映射表在积压消息推送之前就已反映“s 已订阅”。A 排空期间一个并发的 `deliver` 会在映射表中看到 A 的 `tx`，触发 `push_new`，与 A 的 `drain` 在 `new/` 上竞争——按 §5.1 属于良性。

**没有按 session 的锁**，因此对*同一* session 的两个操作可以真正并发运行（例如两个 `deliver`，或 `deliver` + `ack`）。maildir 的重命名操作提供了 broker 层不具备的按消息原子性。这是刻意的设计：“只锁映射表，让文件系统去串行化 mailbox”。

### 5.6 同一 `session_id` 的多个订阅者——**不支持，后写者胜**

`HashMap<String, Sender>`（`core.rs:28`）每个 session id **最多**容纳一个 sender。没有 `Vec<Sender>` / 扇出。两个客户端用相同的 `agent.session_id` 发送 `Hello` 且都 `Subscribe` 就会冲突：第二个 `subscribe`（`core.rs:60-63`）替换第一个。第一个客户端的接收端死掉（§5.2）。**broker 是按 session 单播的总线，不是 pub/sub 主题。**任何“多个 worker 共享一个 session id”的部署都会看到一个 worker 被饿死。这与 `lib.rs:10-12` 的“单一中央 orchestrator + 具名 worker”拓扑一致，但**没有被强制**——两个 worker 可以用相同 id `Hello`，broker 不会拒绝第二个（没有 `BrokerError::Protocol("session in use")`）。第一个订阅者只是静默失去流。

### 5.7 消息丢失场景——汇总

真正丢失一条消息（既未投递也不可恢复）需要满足：
1. **先确认后崩溃（效果未落地）：**`ack` 在 `tokio::fs::remove_file`（`mailbox.rs:210-211`）之后返回 `Ok(())`。如果 OS 的删除还停留在页缓存中机器就崩溃，文件可能重新出现——这是至少一次，不是丢失。真正的丢失需要一个会丢掉已确认删除的文件系统，这超出了 broker 的契约。
2. **路径穿越覆盖：**`mailbox("a/../b")` 会解析为 `<root>/mailboxes/b`。配合 `to` 或 `session_id` 中的 `..`，攻击者（或有 bug 的调用方）可以把消息路由进另一个 session 的 mailbox，然后确认掉。严格说不是“丢失”，而是误路由。`core.rs` 中**没有** `session_id` 校验——见 `core.rs:40-42` 和 `core.rs:46`。WS/认证层是唯一的关卡。
3. **`corrupt/` 隔离：**`recover`（`mailbox.rs:231-232`）把无法解析的 `cur/` 文件移入 `corrupt/` 并**不**推送。因此一条在磁盘上 JSON 损坏的消息（部分写入——有 `atomic_write` 应当不可能，但非原子的文件系统移动或外部篡改可能造成）从订阅者的视角被静默吞掉。它没有被删除（留在 `corrupt/` 中），运维可以恢复，但消费者永远看不到它。这是唯一一条带内“丢失”路径，而且是设计使然。
4. **`push_new` 在已死接收端上的静默发送丢弃**不是丢失——消息在 `cur/` 中，会被重新取回。

在所分析的代码中不存在其他丢失路径。

### 5.8 顺序保证

- **按 session 的 created_at FIFO：**`Mailbox::deliver` 把文件命名为 `<nanos>-<msgid>.json`（`mailbox.rs:153-155`），`drain`/`recover` 按字典序排序（`mailbox.rs:167`、`mailbox.rs:222`）。因此在一个 session 内，消息按 `created_at` 顺序推送，**无论由哪个生产者在何时投递。**纳秒相同的情况由 `MsgId` 决胜，它近似单调（假定是 ULID/UUID——如 `proto.rs:68` 的 `MsgId::new()`）。
- **跨 session 顺序：**没有。两个 session 完全独立（各自 mailbox、各自 channel）。
- **实时推送顺序：**`push_new` 按排序顺序排空（`mailbox.rs:167`），因此实时推送保持 FIFO。`subscribe` 先推送 `recover`（全部 `cur/`）再推送 `drain`（全部 `new/`），这**只有当**每个 `cur/` 文件都比每个 `new/` 文件旧时才符合时间顺序。这一点成立，因为 `drain` 总是把 new→cur 移动，所以 `cur/` 中的文件不会晚于 `new/` 中最旧的文件被投递。20 位纳秒前缀使其严格成立，除非 broker 重启之间时钟倒退。

### 5.9 活性 / 背压

- `mpsc::UnboundedSender`（`core.rs:28`）意味着对慢速消费者**没有背压**。停止调用 `recv` 的订阅者会让它的 channel 无限增长；broker 从不阻塞。对停滞的消费者这是一个 OOM 向量。代价换来的是：向慢速 session 的 `deliver` 永远不会阻塞生产者（对枢纽辐射型 orchestrator 模式有利）。换成有界 channel 会引入背压，但也会让 `push_new` 的 `let _ = tx.send(...)` 以新的方式变得有损。
- 无界 channel 也解释了为什么 `subscribe` 能在函数内部（`core.rs:68-73`）同步完成全部积压推送而不会死锁——它不可能在 send 上阻塞。

---

## 6. 横切观察与风险

1. **没有 session id 校验**（`core.rs:40-42`、`core.rs:46`、`core.rs:55`、`core.rs:84`）。路径穿越（`..`）、空串和超长 id 都会未经清洗地到达 `Mailbox::at`。broker 完全信任 WS/认证层。如果 `serve`/`server` 转发 `ClientFrame::Deliver { to, .. }` 时不校验 `to`，一个已认证的客户端就能写入任意 session 的 mailbox（包括 `ORCHESTRATOR_ID`）。**建议：**在 `core.rs` 中加一个 `fn valid_session_id(s: &str) -> bool`，在 `deliver`/`subscribe`/`ack` 开头调用，返回 `BrokerError::Protocol("bad session id")`。
2. **没有“session 已占用”拒绝**（`core.rs:60-63`）。静默取代（§5.2、§5.6）在运维上令人意外。**建议：**`insert` 冲突时，要么返回 `Err(BrokerError::Protocol("session already subscribed"))`，要么在替换前向旧 sender 发送 `BrokerFrame::Error { reason: "superseded" }`。
3. **`from_text` 返回 `serde_json::Result` 而非 `BrokerResult`**（`proto.rs:46`、`proto.rs:55`）。每个调用点都要手写 `.map_err(|e| BrokerError::Protocol(e.to_string()))?`。**建议：**给 `BrokerError` 加 `#[from] serde_json::Error`，或把 `from_text` 改为返回 `BrokerResult<Self>`。
4. **`to_text` 在序列化失败时 panic**（`proto.rs:43-44`、`proto.rs:52-53`）。目前安全是因为输入是自序列化的 DTO，但这是一条类型系统没有强制的约束。**建议：**（在修好第 3 条之后）返回 `BrokerResult<String>`，或至少在 `InboxMessage` 上文档化这条约束。
5. **未知 `kind` 标签没有前向兼容。**`serde(tag = "kind")` 会拒绝未知变体。滚动部署中 broker 发出新的 `BrokerFrame` 变体会让旧客户端硬性崩溃。**建议：**如果计划滚动部署，加 `#[serde(other)]` 式的兜底或版本字段。（serde 不支持在带数据的内部打标枚举上用 `#[serde(other)]`——需要 untagged 兜底或包一层。）
6. **`ORCHESTRATOR_ID` 是单例**（`lib.rs:32`）。同一 maildir 根上的两个 broker 进程会互相破坏对方的 `cur/`/`new/`。设计假设每个根一个 broker；这一点在 `core.rs` 中没有说明。**建议：**在 `BrokerCore::new` 上文档化“每根一 broker”的约束。
7. **`deliver` 返回 `MsgId`，而 `Delivered` 回执携带同一个 id**（`core.rs:49`、`proto.rs:39`）。这个 id 是*发送方的* `msg.id`（从 `InboxMessage` 透传），不是 broker 分配的 id。所以回执其实只是“是的，你的消息 id X 已入队”。没问题，但值得注意 broker 不做任何 id 分配——按 `MsgId` 去重完全是消费者的工作。
8. **`subscribe` 中的 `tx.clone()` 是冗余的**（`core.rs:63`）。局部 `tx` 可以直接 move 进映射表（channel 已创建，调用方只需要 `rx`）。克隆无害，但暗示作者可能曾打算保留一个本地 sender。小问题。

---

## 7. 文件级汇总表

| 文件 | 行数（不含测试） | 公开项 | 角色 |
|---|---|---|---|
| `error.rs` | 21 | `BrokerError`（4 个变体）、`BrokerResult` | 错误面；只有 `Store` 带 `#[from]`。 |
| `proto.rs` | 58（不含测试） | `ClientFrame`（4 变体）、`BrokerFrame`（4 变体）、`to_text`/`from_text` ×2 | 线上 DTO；原样复用 `bamboo_subagent` 类型。 |
| `core.rs` | 112（不含测试） | `BrokerCore`、`new/deliver/subscribe/unsubscribe/ack/is_subscribed`、`push_new`（私有） | 与传输无关的路由引擎；基于 maildir。 |
| `lib.rs` | 45 | 8 个公开模块、`ORCHESTRATOR_ID`、约 20 个再导出 | 门面；记录枢纽辐射拓扑。 |

报告完。
