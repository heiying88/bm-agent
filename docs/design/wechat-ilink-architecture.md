# 微信 iLink 渠道架构：网关与智能体的关系

> 本文解释微信个人号（iLink 协议）渠道里各组件的角色分工与数据流，
> 面向需要理解/维护这条链路的开发者。实施细节见
> [`wechat-ilink-adapter-plan.md`](wechat-ilink-adapter-plan.md)，
> 使用配置见 [`../guides/CONNECT.md`](../guides/CONNECT.md) 第 4 节。

## 1. 全景图

```
你的手机微信                腾讯的网关                    你的电脑（本地）
┌──────────┐   ┌─────────────────────────┐   ┌──────────────────────────────────┐
│ 微信客户端 │ ⇄ │  iLink 网关              │ ⇄ │  bamboo serve 进程                │
│          │   │  ilinkai.weixin.qq.com  │   │ ┌────────────────────────────┐    │
└──────────┘   │                         │   │ │ wechat 适配器               │    │
               │  · 暂存发往 bot 的消息    │   │ │   纯协议翻译，不懂 AI        │    │
               │  · 派发 bot 的回复       │   │ ├────────────────────────────┤    │
               │  · context_token 路由    │   │ │ ConnectBridge（总机）        │    │
               │  · bot_token 鉴权        │   │ │   会话路由/忙锁/队列/审批     │    │
               └─────────────────────────┘   │ ├────────────────────────────┤    │
                          ↑                   │ │ bamboo-engine（智能体本体）  │    │
                    只传聊天文本，               │ │   agent 循环 + 19 个工具     │    │
                    不碰你本地任何东西            │ │   记忆/压缩/技能/MCP         │    │
                                             │ ├────────────────────────────┤    │
                                             │ │  LLM provider（anthropic…） │ ← 大脑的模型服务
                                             │ └────────────────────────────┘    │
                                             │ 会话/凭据/记忆全在 ~/.bamboo 本地  │
                                             └──────────────────────────────────┘
```

链路一句话：**微信 ⇄ 腾讯网关（邮局）⇄ 本地适配器（翻译官）⇄ Bridge（总机）⇄ Engine（大脑）⇄ LLM（模型服务）**。

## 2. 各层职责与边界

| 层 | 位置 | 是什么 | 不是什么 |
|---|---|---|---|
| iLink 网关 | 腾讯侧 `ilinkai.weixin.qq.com` | 消息中转站：暂存发往 bot 的消息、投递 bot 的回复、用 `context_token` 做会话路由、用 `bot_token` 鉴权 | 不是智能体，不理解消息内容，不执行任何逻辑 |
| wechat 适配器 | `crates/app/bamboo-server/src/connect/platforms/wechat.rs` | 协议翻译官：iLink JSON ⇄ 统一消息类型；管长轮询游标、每会话限流、错误脱敏、`ret=-14` 扫码重登 | 不做任何智能决策，不感知会话语义 |
| ConnectBridge | `connect/bridge.rs` | 总机：按会话键路由、忙锁 + FIFO 排队、`/new` `/stop` `/status` 命令、审批挂起与放行 | 平台无关——telegram/飞书/微信共用同一套 |
| bamboo-engine | `crates/engine/bamboo-engine` | 智能体本体：agent 循环、工具执行、记忆与上下文压缩、技能与 MCP | 不直接接触任何 IM 协议 |
| LLM provider | 外部（anthropic/openai/…） | 模型推理服务，被 engine 调用 | 不落任何你的数据（凭据在本地加密保存） |

**扫码登录的本质**：把你这个微信号在网关侧注册成一个"bot 身份"，网关签发
`bot_token`；此后凭它收发。`context_token` 机制同时实现了协议的根本约束——
**机器人只能被动回复，不能主动发起**（每次回复都必须附在对方某条消息的
上下文上，网关才知道把回复挂到哪个对话、投给谁）。

## 3. 一条消息的完整旅程

以"帮我看看这个目录"为例：

1. **入站**：微信客户端 → 微信服务器 → iLink 网关暂存该消息；
2. **拉取**：本地适配器正挂着一个约 35 秒的 `getupdates` 长轮询请求，网关
   立即返回消息（游标 `get_updates_buf` 随之推进并落盘
   `~/.bamboo/connect_wechat/cursor.json`，保证重启不重复消费）；
3. **翻译**：适配器把 JSON 翻成统一的 `InboundMessage`
   （`chat_id`/`user_id` = 发送者微信 id，`reply_ctx` = `to_user_id` +
   `context_token`；出站回显 `message_type=2` 在此被过滤，防止自回复死循环）。
   入站媒体也在这层处理：**语音条**取微信自带的 ASR 转写文本（`[语音] ...`）；
   **图片**从 CDN（`novac2c.cdn.weixin.qq.com`）下载密文、AES-128-ECB 解密、
   落盘到 `connect_wechat/media/`，并以 `[图片] <路径>` 告知 Agent（其图像
   工具可直接打开该文件）；
4. **路由**：bridge 校验 `allow_from`（空列表全拒）→ 按
   `platform:message_id` 去重 → 查 `wechat:<chat_id>:<user_id>` 对应的
   Bamboo 会话（没有则新建）→ 若该会话空闲则启动一次 agent run，
   忙则进 FIFO 队列串行处理；
5. **执行**：engine 带着会话上下文调用 LLM，按需在你**本地**执行工具
   （读写文件、shell、搜索……），产生事件流；
6. **整形**：render 层按微信声明的能力（无按钮、不可编辑）把事件流整形为
   文本，超长自动分条（2000 字符/条）；
7. **出站**：适配器按 1 条/秒的每会话限流，带着那条消息的 `context_token`
   逐条调 `sendmessage` → 网关 → 微信收到回复。

**审批插曲**（第 5 步中若工具需要授权）：run 挂起 → 微信收到编号文本列表
（"1. 允许 / 2. 拒绝"）→ 你回复"1"或"允许" → 走 ask 快路径**跳过排队**直接
放行 → run 继续。

## 4. 状态都存在哪

| 状态 | 位置 | 说明 |
|---|---|---|
| bot_token | `~/.bamboo/connect.json`（静态加密） | 网关身份；`ret=-14` 过期后由扫码重登更换（新 token 先在内存，建议抄回配置） |
| 轮询游标 | `~/.bamboo/connect_wechat/cursor.json` | 决定"下次从哪继续取消息"；丢失会导致消息重放（有去重兜底） |
| 会话映射 | `~/.bamboo/connect_sessions.json` | bridge 维护的 `会话键 → Bamboo session` 持久化表 |
| Bamboo 会话本体 | `~/.bamboo`（会话存储 v3） | 完整对话与工具记录，与其他渠道共享同一套 |
| context_token | 仅内存（挂在每条入站消息上） | 网关会话路由凭据；有效期未公开，过期表现为发送失败 |

## 5. 数据边界（本地优先的部分）

- **会经过腾讯网关的**：聊天文本本身（TLS 传输，内容对腾讯侧可见——这是
  接入微信的固有代价）；
- **永远留在本地的**：会话记录、凭据（静态加密）、记忆（`~/.jiandu`）、
  工具执行的文件与命令、审批决策。网关碰不到其中任何一项；
- 配置不热更新：bridge 与适配器只在 `bamboo serve` 启动时读一次
  `connect.json`，改配置需重启。

## 6. 异常路径的行为

| 异常 | 行为 |
|---|---|
| 网络抖动 / getupdates 失败 | 5 秒退避后重试，消息不丢（游标未推进网关会重发） |
| `ret=-14` 会话过期 | 自动进入扫码重登：二维码写到 `connect_wechat/login_qr.png` 并打日志，扫码确认后恢复轮询；重登失败每 5 分钟再挂一次二维码 |
| 服务重启 | 游标 + 会话映射 + 会话本体全部从磁盘恢复；重启期间网关暂存的消息会在恢复后送达 |
| 你在 agent 运行中连发多条 | 忙锁 + FIFO 排队，逐条处理 |
| 发送方不在 allow_from | 静默拒绝（启动日志有警告），日志可查到被拒的微信 id |

## 7. 与其他渠道的同构性

微信适配器与 telegram 适配器同构（纯 HTTP 长轮询、无公网依赖），与飞书
适配器共享同一 bridge/render/approvals 层。三者的差异只在"翻译官"这一层：

| | telegram | 飞书 | 微信（iLink） |
|---|---|---|---|
| 传输 | HTTPS 长轮询 | WebSocket 长连接 | HTTPS 长轮询 |
| 回复路由 | chat_id | open_id + 卡片 | `context_token`（只被动回复） |
| 消息编辑/按钮 | 支持 | 支持（卡片） | 均不支持（文本分条 + 编号审批） |
| 凭据 | bot token | app_id + secret | bot token（扫码获取） |

## 8. 代码索引

| 关注点 | 文件 |
|---|---|
| 协议翻译 / 长轮询 / 扫码重登 | `crates/app/bamboo-server/src/connect/platforms/wechat.rs` |
| 平台无关抽象（trait 与消息类型） | `crates/app/bamboo-server/src/connect/platform.rs` |
| 会话路由 / 排队 / 命令 / 审批挂起 | `crates/app/bamboo-server/src/connect/bridge.rs`、`approvals.rs` |
| 事件流 → 平台消息整形 | `crates/app/bamboo-server/src/connect/render.rs` |
| 注册与启动接线 | `crates/app/bamboo-server/src/connect/mod.rs` |
| 扫码取 token 脚本 | `scripts/wechat-login.ps1` |
