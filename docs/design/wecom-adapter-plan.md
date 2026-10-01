# 微信渠道适配器方案 —— 企业微信智能机器人（长连接）

> 状态：**后续渠道**（2026-10-01 用户拍板：首个微信通道走个人号官方 iLink 协议，
> 见 [`wechat-ilink-adapter-plan.md`](wechat-ilink-adapter-plan.md)；本方案保留为企业微信
> 后续接入的既定蓝图）。目标是为 Bamboo connect 子系统新增第三个渠道适配器 `wecom`，
> 让用户可以在微信生态（企业微信客户端）里操控 Bamboo Agent。
> 结构上参照 `feishu-adapter-plan.md`（本仓库第 2 个适配器的蓝图）与飞书适配器的实际落地代码。

## 0. 选型结论（为什么是企业微信智能机器人）

| 候选路径 | 结论 | 理由 |
|---|---|---|
| **企业微信「智能机器人」长连接** | ✅ **推荐** | 官方 API；`wss://` WebSocket 长连接，**无需公网 IP/域名/备案/反向代理**；认证仅需 BotID + Secret；支持流式回复；协议与飞书长连接模式同构，本仓库已有 940 行的成熟模板（`feishu/ws.rs`） |
| 企业微信自建应用回调模式 | 备选 | 需要公网 HTTPS 回调端点（GET echostr 验签 + POST AES 加密 XML）。connect 子系统目前**没有任何入站 HTTP 面**，需要把 `spawn_platform_tasks` 内部创建的 channel sender 上提重构才能被 actix handler 触达——改动大、运维门槛高 |
| 微信公众号（测试号/正式号） | 备选 | 同样是回调模式（需公网）；正式号需认证；被动回复有 48 小时窗口限制 |
| 个人微信号（wechaty / Gewe 等第三方 Hook 协议） | ❌ 不做 | 无官方 API，违反微信服务条款，**有封号风险**（Hook 直连已有实测封号案例）；且主流 puppet 多为付费 |

### 0.1 补充（2026-10）：OpenClaw 桥接路径与个人微信现状

个人微信接入在 2026 年出现官方化转折，评估结论如下（详细来源见会话记录）：

- **腾讯官方插件 `@tencent-weixin/openclaw-weixin`**（腾讯微信团队维护，扫码登录，支持私信/媒体）
  已成为个人微信目前最正规的接入面；2026-03 腾讯官方推出 ClawBot（agent 直接出现在微信聊天列表）。
  Hook/逆向直连仍有明确封号案例，维持不做。
- **OpenClaw 桥接用法**：OpenClaw 是与 Bamboo 同类的自部署 agent 网关，但可配置自定义
  OpenAI 兼容端点作为模型后端，而 Bamboo 内置 `/openai/v1/chat/completions` 兼容端点
  （跑完整 Bamboo agent 循环）。因此存在零 Rust 改动的组合：
  `个人微信 ⇄ OpenClaw（+官方微信插件）⇄ http://127.0.0.1:9562/openai/v1`。
  - 优点：半天成本即可在个人微信里体验 Bamboo 大脑；记忆/工具/技能全在 Bamboo 侧保留。
  - 代价：常驻一个 Node 网关进程；OpenClaw 包了一层 agent 语义，connect 子系统的
    通道内命令（`/new` `/stop` `/status`）与审批按钮交互**不可用**；OpenAI 兼容端点与
    OpenClaw 会话的映射关系需 spike 验证（一个 OpenClaw 对话是否稳定对应一个 Bamboo
    session）。
- **定位**：OpenClaw 桥接是"先跑通体验"的临时通道；**正式通道仍是本方案的 wecom 适配器**。
  若后续确有个人微信刚需，可再评估基于官方 iLink 协议的原生 connect 适配器（观察项，
  等协议文档与稳定性成熟后立项）。

关键事实（调研确认）：

- 官方协议文档：[智能机器人长连接](https://developer.work.weixin.qq.com/document/path/101463)（`wss://` + `aibot_subscribe` 订阅；每个机器人同一时间只允许一条有效连接，新连接完成订阅后踢掉旧连接）。
- 消息接收与流式回复：[接收消息（智能机器人）](https://developer.work.weixin.qq.com/document/path/100719)。
- 机器人创建与授权：企业管理后台「管理工具 → 智能机器人 → API 模式管理」授权成员创建（[帮助文档](https://open.work.weixin.qq.com/help2/pc/21669)）；个人可免费注册企业微信组织，零成本。
- 长连接由开发者服务器主动发起，底层 `wss://` 自带加密，业务层无需额外加解密（对比回调模式的 AES+XML）。

## 1. 免费继承的通用件（不需要写一行代码）

以下是 connect 子系统平台无关、`wecom` 直接复用的部分（与飞书方案 §1 同源）：

- `connect/mod.rs` 的 `dispatch_loop`（mod.rs:312-340）把 `Inbound::Message/Callback` 路由进 bridge；
- `bridge.rs` 全部语义：`SessionKey = platform:chat_id:user_id`、`allow_from` 精确 user_id 匹配（空列表 = 全拒）、`"{platform}:{message_id}"` 去重（容量 10_000）、过期丢弃（`sent_at < process_start`）、busy 锁 + FIFO 队列、`/new` `/stop` `/status` 命令、ask 快路径；
- `render.rs`：按 `capabilities().edit_message` 选模式——**wecom 声明 `edit_message=false` 即自动走 legacy 渲染**（每个工具事件一条 + 最终文本 `chunk_message` 分条），编辑失败降级路径天然用不到；
- `approvals.rs`：无按钮平台是**一等公民**——`Capabilities::buttons=false` 时 `render_ask` 只发编号文本列表，`match_text_answer` 的中文意图关键词（"允许/同意/确定/是/拒绝/不/否"）开箱即用；`answer_callback` 用 trait 默认 no-op（platform.rs:179-185）；
- 引擎侧 `EngineResponder` / `ConnectResumePort`（submit + resume + 权限授予重放）完全平台无关。

## 2. 需要动手的部分

### 2a. 配置与秘密：**零加密管道改动**（推荐路径）

`ConnectPlatformConfig`（`bamboo-config/src/config.rs:1159-1240`）已有 `app_id`（明文）+
`app_secret`（`skip_serializing` + `token/app_secret_encrypted` + credential store 引用）结构，
语义与智能机器人的 **BotID + Secret** 完全同构。推荐直接复用：

```json
{
  "type": "wecom",
  "app_id": "BotID（智能机器人的 bot id）",
  "app_secret": "Secret",
  "allow_from": ["企业微信成员 userid"]
}
```

- 复用 `app_id`/`app_secret` 字段 ⇒ **秘密管道的"五处往返"全部免改**
  （`config_crypto.rs` 的 hydrate/refresh/sanitize、`patch.rs` 的 intent/清密/掩码保留、
  settings 载荷的 `CredentialAction`）；
- `domain` 字段留空（单一天然域名 `qyapi.weixin.qq.com`，不做三态解析）；
- 代价：字段名语义有轻微借用（文档写清楚即可）。若坚持显式字段名 `bot_id`，则需走一遍
  飞书 §2a 清单里的全部秘密管道触点——可作为后续重构，不阻塞首版。

`multi_bot_guard`（connect/mod.rs:259-277）与 `platform_config_will_start`（mod.rs:284-303）
各加一个 `"wecom"` 谓词（凭据非空判定，照抄 feishu 臂）。

### 2b. 注册分支

`ConnectManager::start` 主 match（connect/mod.rs:70-174）加：

```rust
"wecom" => { /* 校验 app_id/app_secret 非空；构造 WecomPlatform；spawn_platform_tasks(...) */ }
```

`connect/platforms/mod.rs` 加 `pub mod wecom;`。路由/HTTP API 无需改动
（`GET/PUT /bamboo/config/connect` 平台无关）。

### 2c. 适配器本体（预计 ~1500 行含测试，介于 telegram 1111 与 feishu 2800 之间）

```
connect/platforms/wecom/
  mod.rs   WecomPlatform：Platform impl（6 方法）、入站事件 -> Inbound 映射、
           群聊 @ 提及门控（仿 feishu strip_mentions / passes_group_gate）
  ws.rs    wss 长连接客户端：bootstrap(aibot_subscribe) / 心跳 / 事件解包 / 重连退避
           （结构仿 feishu/ws.rs，但协议更简单——无多分片重组、无帧级 ack oneshot）
  api.rs   TokenCache（access_token 单飞刷新 + 失效重试一次）、发送/流式回复 REST、
           RateLimiter（复制 telegram/feishu 同形实现）、sanitize_error（秘密不进错误文本）
```

要点：

- **Capabilities**：`{ buttons: false, edit_message: false, images: false, files: false }`
  （首版纯文本；智能机器人消息能否编辑以 spike 实测为准，能编辑再升级流式就地编辑）；
- **chat_id 语义**：单聊 = 对端 userid；群聊 = 会话 roomid（保证 SessionKey 三段唯一）；
- **消息长度**：WeCom 智能机器人 content 上限以实测为准，`chunk_message` 的 limit
  参数化传入（render.rs 共享函数按字符切分，不断 UTF-8，直接可用）；
- **回复机制（spike 重点）**：长连接模式下收到消息后，回复走 WS 帧 ack + 携带临时
  凭据的 REST 流式消息（文档 100719 的"流式消息回复"），token 生命周期与刷新节奏需要
  真机验证后固化到 `api.rs`；
- **单连接语义**：协议规定一机器人一连接、新连接踢旧连接——`ws.rs` 重连逻辑要容忍
  "被自己踢"的场景（重启窗口期），退避后重订即可。

### 2d. 推迟到后续

- 模板卡片/按钮交互（若智能机器人后续支持，可升级 `buttons: true`）；
- 回调（webhook）模式作为长连接的备胎（需要入站 HTTP 面重构，见 §0）；
- 多机器人：用 platform 字符串折叠 `"wecom:{bot_id}"` 的廉价消歧方案（同 feishu 计划 §2d），
  首版维持 `multi_bot_guard` 单 bot 限制；
- 图片/文件收发。

## 3. 实施顺序

1. **配置与注册骨架**：2a + 2b + `multi_bot_guard`/`platform_config_will_start` 谓词 + 单测
   （照抄 mod.rs:342-464 的注册臂测试模板）。验收：`connect.json` 写入 wecom 段后启动
   有预期 warn/错误，无 panic。
2. **出站半边**：`api.rs`（TokenCache/发送/限流/脱敏）+ wiremock 单测
   （镜像 telegram.rs:555-1111 的 stub 模式，含
   `transport_errors_never_leak_the_bot_token` 式秘密泄露断言）。
   验收：单测绿；curl 手测能发消息。
3. **长连接传输**：`ws.rs` 订阅/心跳/重连 + 纯函数单测（帧解析先于联网调试）。
4. **组装 + 真机 spike**：`mod.rs` Platform impl 接通两端；企业微信 4.x 管理后台创建
   智能机器人（API 模式）→ BotID/Secret → 真机收发 + allow_from 生效验证 + 流式回复
   协议固化。验收：微信里发"你好"收到 Agent 回复；`/new` `/stop` `/status` 可用；
   权限请求以编号文本出现，回复"1/允许"可授权。
5. **文档与收尾**：`docs/guides/CONNECT.md` 增加企业微信小节（中文，与现有两平台同风格）；
   补 `docs/design/` 本文档状态更新；`CHANGELOG` 记录。

## 4. 前置条件（使用侧）

- 企业微信管理后台权限：「管理工具 → 智能机器人 → API 模式管理」被授权创建机器人；
- 创建智能机器人后取得 BotID + Secret；
- 无需公网 IP / 域名 / 证书（长连接由 Bamboo 主动外连）；
- `allow_from` 填企业微信成员 userid（空列表 = 全拒，启动时 warn）。

## 5. 风险与未决（spike 清单）

| 项 | 风险 | 缓解 |
|---|---|---|
| 流式回复 token 生命周期 | 协议细节文档简略 | 阶段 4 真机验证后再固化；失败则降级为"完成后整条回复"（legacy 渲染天然支持） |
| 消息长度/频率上限 | 未知精确值 | `chunk_message` 参数化 + RateLimiter 保守起步 |
| 智能机器人能力差异 | 不同企业微信版本功能不一 | 以 4.x 当前官方文档为准；卡片/按钮列入推迟项 |
| 踢连接语义 | 重启窗口自踢 | 重连退避 + 订阅幂等 |

## 6. 工作量估计

熟悉 Rust 与本代码库的前提下：阶段 1 ≈ 0.5 天，阶段 2 ≈ 1 天，阶段 3 ≈ 1.5 天，
阶段 4 ≈ 1 天（含真机联调），阶段 5 ≈ 0.5 天，**合计约 4-5 个工作日**。
