# 远程 Mailbox Broker + 跨网络的 `ask_agent`

> 取代 `ask-agent-design.md` 的*传输*部分。建立在 Change A（仅 actor 子代理）之上，
> 延续 `remote-actor-plan.md`（远程 actor）。
>
> **已定决策**：ask/reply 传输 = **独立网络 broker**（`bamboo broker serve`）、
> **WebSocket 推送**、经既有 **`Mailbox` maildir** 持久化、**Bearer** 鉴权；同时现在
> 就铺设 **remote-actor-plan.md 的 P0 接缝**；目标不活跃 → **自动激活**；应答模式 =
> **`query` + `steer`**。

> 下文 Phase 0–2 流程与历史 SHIPPED 章节描述的是最初的 broker-agent 设计。本文的
> #791 边界以 2026-09-29 为准，早期流程与之不一致时以此为准。

## #791 当前边界（2026-09-29）

- broker 仍持有持久 Maildir 传输。作用域 peer 依据运维者 `PeerPolicy` 认证；其连接
  代际与受信任的能力/容量观测汇入 `FileHostRegistry`。worker 自报的角色、mailbox
  或容量永远不构成逻辑 Actor 权威。持久 `ActorSession` 是规范 `Session.id` 与历史；
  `ActorActivation` 与 `WorkerHost` 是可替换的执行状态。
- 对规范的 Child Run，Event/Outcome 帧本身不是完成回执。Host 验证当前 activation
  并保存规范 Child transcript/状态，用其私有 `broker-terminal-receipts.v1.json`
  对该确切 Run 的 transcript 前缀做预备、提交、恢复与守护；随后
  `AckWithReceipt`/`AckResult` 在同一持久 broker 命名空间确认删除，Host 才清理该
  回执。状态不确定的旧 Run 会阻断后继消费，而不是被计为已完成的 Run。
- `SubAgent` 是逻辑 Child 接口，覆盖创建、纠偏、检查、控制、直接父级
  `ParentRequest` 回复，以及经 `ParentQuestion` 的 child 澄清。其有界 diagnostics
  暴露队列、租约、dead-letter、activation 与待处理请求状态，不暴露 worker 凭证
  或载荷。旧 `ask_agent` 工具保留为兼容路由：精确的直接拥有的规范 Child ID 仅支持
  持久的 `status`/`progress` 查询；规范 Child 的转向用 `SubAgent target`。物理
  cluster-worker 的 query/steer 保留 broker 语义。旧 `ask_agent`/`deploy_agent`
  不进入模型常规工具目录。
- 固定远端的 scoped WSS `Run` 与 `EnvironmentLease` v1 路径已存在。Host 为确切
  Run 捕获干净 Git workspace 的身份；Worker 在执行前核验自己的 checkout。broker
  认证的容量观测与 slot 预留已就绪，但其自身不授权任何 Run。
- 通用远端 Run 编排、跨 Host 迁移/故障转移、更广的环境获取/隔离，以及剩余的回执
  活性修复**尚未交付**。回执或 ACK 证据不确定时，当前恢复刻意保持阻断
  （fail closed）；不要把固定远端路径读作这些更宽契约的完成。

为什么不用共享目录下的文件 mailbox：它仅限本机文件系统，够不到远程 worker。broker
把持久 `Mailbox` 放到网络端点之后，父端与 worker——本地或远程——都经 WS 访问它，
持久性与远程可达一并获得。

---

## Phase 0 —— 远程 worker 接缝（= remote-actor-plan.md §6 P0，零行为变化）

把三个本地死结抽象成 trait；默认 impl 逐行复刻现状。（细节 + 行号引用见
`remote-actor-plan.md` §2.2/§3。）

1. `WorkerLauncher` trait + `LocalSubprocessLauncher` —— 封装 `fleet.rs::spawn_worker`。
   返回 `LaunchedWorker { client: ChildClient, kill_handle: Option<…> }`。
2. `Discovery` trait + `FileFabric` —— 封装 `discovery.rs::Fabric`
   （publish/resolve/discover/withdraw/gc）。
3. `WsServer::bind(addr)` / `bind_tls(addr, identity)` —— `bind_loopback` 保持默认。
4. `Placement` 枚举（`Local` | `Remote{endpoint}` | `Schedulable{pool}`）加入
   `ProvisionSpec`（`serde(default)=Local`，前向兼容）。
- **验收**：`subagent_actor_via_server.rs`、`subagent_worker_e2e.rs`、
  `e2e_subprocess.rs` 全绿；`cargo test -p bamboo-subagent` 通过；零行为变化。

正是这些接缝让 broker（Phase 1）能在本地或远程放置/连接 worker，而 ask 路径不必
关心它们跑在哪。

---

## Phase 1 —— `bamboo broker serve`（独立 WS broker）

### 进程与线协议
- 新增隐藏子命令 `bamboo broker serve --bind <addr> [--tls] --token <T>`
  （在 `src/bin/bamboo.rs` 中镜像 `subagent-worker` 的注册）。
- WebSocket 服务端（`WsServer::bind`/`bind_tls`）。每个客户端（父端或 worker）
  一条 WS 连接。Bearer token 放 WS 握手子协议 / 首帧（复用
  `ProvisionSpec.secrets` 的 scoped-envelope 纪律——token 绝不进 argv/env）。

### 身份与寻址
- 每个客户端连接时表明身份：`Hello { agent_ref: AgentRef, token }`，其中
  `AgentRef.session_id` 即 mailbox 键。broker 在自己的 root 下为每个
  `session_id` 持有一个 `Mailbox`：
  `<broker_root>/mailboxes/<session_id>/{new,cur,corrupt}`。

### Broker 帧（新增 `broker::proto`）
```
ClientFrame（客户端 → broker）:
  Hello { agent_ref, token }
  Deliver { to: session_id, message: InboxMessage }   // 入队到 to 的 mailbox
  Subscribe                                            // 开始接收自己的 mailbox
  Ack { id: MsgId }                                    // 从 cur/ 删除
BrokerFrame（broker → 客户端）:
  Welcome { } | Error { reason }
  Message { message: InboxMessage }                    // 从自己的 mailbox 推送（new/→cur/）
  Delivered { id }                                     // 投递回执
```
- **持久性**：`Deliver` 先做 `Mailbox::deliver`（temp+rename 原子写）再向发送方
  确认——broker 重启也能存活。`Subscribe` 时，broker 先对 `cur/` 执行
  `recover()` 再 drain `new/`，推送 `Message` 帧；消费者 `Ack` 后删除。
  at-least-once；经 `AdmittedSet` 去重（已存在）。
- **推送**：broker 盯住每个已订阅的 mailbox（对该键的 `Deliver` 通知 + 周期清扫）
  并及时推送——客户端无需轮询。

### 复用
- `InboxMessage` / `InboxKind::{Ask,Reply}` / `AskBody` / `ReplyBody` /
  `correlation_id`（Layer 1，已建成）原样作为 broker 的消息 schema。
- `Mailbox` maildir 原样作为 broker 的存储。

---

## Phase 2 —— 经 broker 的 `ask_agent`

流程（目标 = 调用方自己的 child / 常驻 agent；作用域守卫 = send_message 的
Root 调用方 + `load_child_for_parent`）：
```
SubAgent action=ask
  └─ ask_child_action(parent, target, question, mode, timeout)
       └─ broker.Deliver { to: child_id, Ask{ question, mode }, from: parent, id: ask_id }
       └─ 目标不活跃 → 自动激活:
            Placement::Local  → LocalSubprocessLauncher (spawn worker；worker 启动后即
                                 Subscribe 到 broker 并 drain 排队的 Ask)
            Placement::Remote → ConnectLauncher 连远端 endpoint (P1)
       └─ 在父端自己的 broker 订阅上等待 BrokerFrame::Message{ Reply{answer},
          correlation_id==ask_id }，以 timeout 为上界 → Ack → 返回 {answer}
```
Worker 侧（连到 broker——替代或叠加父端 WS）：
- 启动时：`Subscribe`；对每个 `Ask`：`query` = 在 session 克隆上做一次临时
  `agent.execute`；`steer` = 作为实时用户轮次注入；收割最终 assistant 消息；向
  `from.session_id`（父端）`Deliver` 一条 `Reply{answer, correlation_id=ask.id}`；
  `Ack` 该 Ask。
- 每个 worker 内 Ask 串行处理（约束 LLM 负载）。

工具：`SubAgentArgs::Ask { child_session_id?|resident_name?, question, mode?, timeout_secs? }`
→ `tool_result({from, answer, mode, status:"answered"})`。`SERVER_TOOL_NAMES`
无变化。

---

## 构建顺序（每步先编译 + 测试，再进下一步）

- **P0.1** `WorkerLauncher` + `LocalSubprocessLauncher`（封装 spawn_worker）。
- **P0.2** `Discovery` + `FileFabric`（封装 Fabric）。
- **P0.3** `WsServer::bind`/`bind_tls`；**P0.4** `Placement` 加入 `ProvisionSpec`。
  → e2e 全绿，零行为变化。
- **B1** broker crate/二进制：帧 + WS 服务端 + Mailbox 承载的路由 + 鉴权 + 测试。
- **B2** broker 客户端（父端适配器与 worker 共用）。
- **B3** worker：subscribe + Ask 处理器（query/steer）+ Reply。
- **B4** engine `ask_child_action` + 移植 + 经 launcher 自动激活。
- **B5** `SubAgent action=ask` + 常驻解析 + 描述 + 测试 + e2e。

---

## 历史 SHIPPED 记录（最初的 broker-agent 切片）

分支 `feat/subagent-actor-only`。全部是在 Change A（仅 actor）+ Phase 0（远程
worker 接缝）之上的增量。以下内容均已实现并测试。

**拓扑（已定）**：单一中心 broker，hub-and-spoke。broker 是*纯消息总线*（路由
Ask/Reply；从不 spawn 或协调其他 broker）。master 部署执行环境（本地子进程 /
Docker / SSH），由它们回拨 broker——推模型，而非相互发现。

**crate `bamboo-broker`**（`crates/app/bamboo-broker`）：
- `proto` —— `ClientFrame`（Hello/Deliver/Subscribe/Ack）↔ `BrokerFrame`（Welcome/Error/Message/Delivered）。
- `core::BrokerCore` —— 按 session 持久的 `Mailbox` 路由、推送订阅、at-least-once。
- `server::BrokerServer` —— WS 总线 + Bearer token 握手（`bamboo broker serve`）。
- `client::BrokerClient` —— connect + 分发（messages / delivered）。
- `serve::serve_executor` —— worker 循环；跑一个 `ChildExecutor` 回答每个 Ask；
  **query** = 在上下文副本上只读，**steer** = 持久写入上下文。兼容 EchoExecutor
  （无 LLM）与真实的 BambooRuntime 执行器。
- `ask::ask_agent` / `ask_over` —— 编排器投递 Ask 并等待关联的 Reply。
- `deploy` —— `Deployer` + `LocalProcessDeployer` / `DockerDeployer` /
  `SshDeployer`（都 spawn `bamboo broker-agent serve …`；token 走 env，绝不进
  argv）。

**可部署 agent**：`bamboo broker-agent serve --broker <ws> --token <t> --id <id> [--echo|--model]`
（`src/broker_agent.rs`）——连接一个 broker 并服务其 mailbox，随处可用。

**循环内指挥工具**：`ask_agent`（`bamboo-server-tools`），仅在配置了
`subagents.broker { endpoint, token }` 时叠加到 Root 工具面。运行中的 root agent
用它指挥另一个经 broker 部署的 agent（query/steer）。SubAgent 工具不变。
（注：实现为独立的 `ask_agent` 工具而非 `SubAgent` action——broker ask 与
SubAgent 的 child-session 端口是不同基底，隔离的叠加工具风险更低；
`OverlayToolExecutor` 按 tool name 路由，故 `SERVER_TOOL_NAMES` 无变化。）

**传输说明**：ask/reply 走 broker 以 WS 为前端的持久 mailbox（可跨远程）；字面的
文件 mailbox 只是 broker 的存储基底。`ParentFrame::DrainAsks`（过渡期的文件
提醒）已弃用。

**测试（记录于那个历史切片，不是当前 #791 的证据）**：broker 14 lib +
ws_roundtrip 3 + serve/ask 集成；`ask_agent_tool` 2；deploy e2e 3（经
LocalProcessDeployer 用真实 `broker-agent --echo` 子进程：单次 query+steer、
双 agent 独立指挥、live-Docker 受门控）。回归：config 110、subagent 41 + e2e 2、
engine 789、server 850、server-tools 26、actor e2e 3（server→真实 worker）、
session_history 4。

**推迟（边界明确）**：`bind_tls`/`wss://` + 远程 `ConnectLauncher`/
`Placement::Remote` 端到端（`remote-actor-plan.md` 的 P1；接缝 + `Placement`
枚举已就位）；真实 LLM 的 broker-agent 路径已接线，但其 e2e 需要 provider
（确定性路径用 `--echo`）；broker 间联邦（明确出范围——已选 hub-and-spoke）。

## Layer-1 对账
保留 `InboxMessage.correlation_id`、`AskMode`、`AskBody`、`ReplyBody`（broker
schema）。接线 B3 时去掉 `ParentFrame::DrainAsks`（文件提醒，在 broker 下已
过时）。

## 开放/可再确认
- broker 作为独立 crate `crates/app/bamboo-broker`（lib）+ `bamboo` 二进制里的子命令。
- v1 单 broker 实例；HA/分片以后再说。
- 发现（`RegistryFabric`）以后也可移到 broker 上（remote-actor-plan.md P2）。
