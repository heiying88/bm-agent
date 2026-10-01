# 飞书适配器方案 —— epic #447 第 3 阶段

2026-07-13 重做的分析（worktree `feat/feishu-adapter`，尚无提交；基于 phase 1 #453 + phase 2 #459 + 配置拆分 #456/#460 + review 跟进 #462 之后的 origin/main）。

## 1. 已有内容（复用，不要动）

通用且与适配器无关——飞书适配器全部继承：

- `connect/mod.rs` 的 `dispatch_loop`（mod.rs:133）把 `Inbound::Message/Callback` 路由进 bridge。
- `bridge.rs`：allow_from 对 `user_id` 精确匹配（bridge.rs:430）、过期丢弃（`sent_at < process_start`，:440）、按 `"{platform}:{message_id}"` 去重（:449）、`SessionKey = platform:chat_id:user_id`（:32）、忙碌锁 + FIFO 队列、`/new` `/stop` `/status`、ask 快路径。
- `render.rs`：按 `capabilities().edit_message` 选模式；流式就地编辑节流 `EDIT_MIN_INTERVAL=1500ms` 且 `EDIT_MIN_NEW_CHARS=30`（render.rs:43）；`chunk_message` + `MAX_MESSAGE_CHARS=4096` 导出给适配器；编辑失败降级为全新回复（绝不让 run 失败）。输出**纯文本**（`OutboundMessage` 无 markdown 标志）。
- `approvals.rs`：`callback_data = "{nonce}:{option_index}"`（nonce = UUID 首段）；带编号的文本列表总是发送，按钮纯属增强；文本回退（序号 / 精确选项 / 中英 yes-no 意图 / 自定义）；`answer_callback` 必须恰好 ack 一次，过期 ⇒ `Some("This action has expired.")`；`EngineResponder` 解析路径与平台无关。
- 测试接缝：适配器单测用 `wiremock`（telegram.rs:539+ 是模板，含 `with_options(token, base_url, tiny_rate_interval)` 注入 + token 绝不泄露测试）；bridge/render 测试用进程内 `FakePlatform`/`RecordingPlatform`——无需新基建。

适配器侧职责（Telegram 先例）：出站限流（按 chat 的 token bucket，telegram.rs:96）、`reply` 内消息分块、每个错误字符串里的秘密脱敏（`sanitize_error`，telegram.rs:177）、共享 `OnceLock` reqwest 客户端。

## 2. 新增工作项

### 2a. 配置 + 秘密管道（最大的横切改动）

`ConnectPlatformConfig`（bamboo-config config.rs:619）目前只有一对 `token`/`token_encrypted`。飞书需要 **app_id（非秘密）+ app_secret（秘密）**。建议：新增可选字段，而不是重载 `token`：

```rust
pub app_id: Option<String>,                 // 明文，正常序列化
pub app_secret: Option<String>,             // skip_serializing，仅内存
pub app_secret_encrypted: Option<String>,   // 落盘态
pub domain: Option<String>,                 // 明文，正常序列化 —— 已决策：必做的配置面
```

**`domain`(已决策,必做)**:默认 `"feishu"` → `https://open.feishu.cn`;`"lark"` → `https://open.larksuite.com`;任意 `https://` 开头的值 → 私有化部署 base URL 原样使用(cc-connect 同款三态语义)。REST 与 WS bootstrap(`/callback/ws/endpoint`)共用同一 base;适配器内只存解析后的 `base_url: String`,构造时归一化(去尾部 `/`),非法值(既不是预设名也不是 https URL)在注册 arm 里 warn+skip 该 entry,与空 token 同路径。非密钥,正常序列化进 connect.json,无需进 secret 管道。

五处秘密往返点同步扩展（#430 契约）：
1. hydrate：`hydrate_connect_platform_tokens_from_encrypted`（config_crypto.rs:509）
2. 保存时再加密：`refresh_connect_platform_tokens_encrypted`（config_crypto.rs:538）
3. GET 脱敏（redaction/mod.rs:161 —— 按 `platforms[i]` 位置对应）
4. PATCH 掩码保留：`preserve_masked_connect_secrets`（patch.rs:422 —— 数组顺序即契约）
5. struct + connect.json 往返测试。

前端绝不能预填 `****...****` 占位符（既有契约）。

### 2b. 注册分支

`mod.rs:68` 的 `match platform_cfg.platform_type.as_str()` 增加 `"feishu" =>` 分支：校验 app_id/app_secret 存在，`allow_from` 为空时告警（等于全拒绝），构造 `FeishuPlatform`，spawn `start()` + 通用 `dispatch_loop`。

### 2c. `platforms/feishu.rs` —— 适配器本体

**传输：事件长连接（WS），无需公网 IP。** 协议仅 SDK 提供（无公开文档）；已验证的形态：

- Bootstrap：`POST https://open.feishu.cn/callback/ws/endpoint`，body 为 PascalCase `{"AppID","AppSecret","ClientAssertion":""}` → `data.URL`（wss，一次性——每次重连都重新获取）+ `ClientConfig{PingInterval(默认2min), ReconnectInterval(固定2min), ReconnectNonce(30s 抖动), ReconnectCount(-1)}`。从 wss URL 的 query 解析 `service_id`——ping 帧里会回显。致命（停止重连）：非 {0,1,1000040343} 的 bootstrap code、握手 `Handshake-Status:403`、`Handshake-Autherrcode:1000040350`（每 app 超过 50 条连接）。
- 线格式：二进制 proto2 `pbbp2.Frame{SeqID,LogID,service,method(0=ctrl/1=data),headers[{key,value}],payload,...}`。Ping = method 0 + header `type:ping`，每个 PingInterval 一次；pong 可能携带回传的 ClientConfig JSON。数据帧：header `type:event`，payload = 明文 schema-2.0 事件 JSON（WS 路径无 encrypt-key）。多分片：`sum`/`seq` header 以 `message_id` 为键，5s TTL，齐全才 ack。**Ack = 在 3 秒内回显同一帧、payload 为 `{"code":200,"headers":null,"data":<base64>}`**，否则服务端重推；对 `card.action.trigger`，base64 的 `data` 携带回调响应 JSON（toast/card）。
- 依赖：vendor `lark-websocket-protobuf`（MIT/Apache，仅 prost 的 Frame/Header）+ 基于 `reqwest + tokio-tungstenite + prost` 的手写客户端（约 1k 行；openlark-client 是参考实现，但要拖一个 18-crate 的 workspace，而且本来也没有内置重连）。**核对 tokio-tungstenite 的 TLS feature 是否符合 native-tls pin（不含 aws-lc-sys）**——当初否掉 `feishu-sdk` crate 正是因为它 pin 了 rustls+aws-lc-rs。先例：github.com/linuxhenhao/beam（Rust，同技术栈，同用例）。
- REST：`tenant_access_token/internal`（7200s；剩余 ≥30min 时飞书返回同一个 token——剩余 <30min 或遇到 99991663/99991661 时刷新），发送 `POST /open-apis/im/v1/messages?receive_id_type=chat_id`（`content` 是经 JSON 转义的字符串；文本 150KB / 卡片 30KB；可选 `uuid` 发送去重），卡片更新 `PATCH /open-apis/im/v1/messages/:message_id`。

**入站映射**（`im.message.receive_v1`，scope `im:message.*`）：
- `chat_id` = `message.chat_id`；`user_id` = `sender.sender_id.open_id`（**allow_from 条目是 open_id**——需写入文档）；`message_id` = `message.message_id`（官方去重指引；消息不用 event_id）；`sent_at` = `message.create_time`（毫秒字符串）；`text` = 解析 `content` JSON `{"text":...}`，剥掉 `@_user_N` mention 占位符。
- `reply_ctx` = `{"chat_id":..., "message_id":...}`（message_id 为以后的线程化回复留口）。
- 群聊门控（适配器侧；bridge 无此概念）：MVP = p2p + 群内必须 @mention（bot open_id 出现在 `mentions`），cc-connect 先例。丢弃 `sender_type:"bot"`。
- 卡片回调（`card.action.trigger`）：`user_id` = `operator.open_id`，`chat_id` = `context.open_chat_id`，`data` = `action.value` 原样往返（把 bamboo 的 `"{nonce}:{index}"` 放进 `value`），按 `event_id` 去重（这里的 create_time 是**微秒**，消息是毫秒）。

**能力**：`{buttons:true, edit_message:true, images:false, files:false}`。

**cc-connect 飞书源码验证过的坑** (github.com/chenhg5/cc-connect `platform/feishu/`, 2026-07-13 main):
- **同 app 只能开一条 WS**: Feishu 对同一 app 的多条长连接做随机负载均衡(每个事件只发给一条连接),cc-connect 为此实现了 sharedWSGroup(首个实例持有连接、事件扇出给同 app 兄弟)。bamboo 一个 config entry = 一个连接,天然安全;但绝不能在重连时短暂并存两条连接处理逻辑。
- Bot 自身 open_id 需启动时取一次 `GET /open-apis/bot/v3/info`(@mention 判定用);失败则降级为不做群过滤 + warn。
- `@所有人` 消息 mentions 数组为空,文本含 `@_all` —— 单独的 substring 判断。
- 文本里的 mention 是 `@_user_N` 占位符,按 `mentions[].key` 替换:bot 自己的删掉、他人替换为 `@显示名`。
- msg_type 决策树:含 `<at>` 标签或纯文本 → `text`(mention 事件只在 text 消息触发);markdown 表格 >5 个 → `post`(card 超 5 表报 11310);其余 → `interactive` card。
- 回调响应最佳实践:同步返回**替换后的卡片**(按钮消失 = 天然防双击),决定后的卡片内容(label/颜色/正文)直接塞在按钮 `value` 里,回调无需查状态。
- 999916 63 invalid-token → 禁缓存强刷 token 重试一次;所有发送包 3 次指数退避 transient 重试(仅网络类错误)。
- 流式 PUT 撞 230020 限流时直接丢帧(下一次 flush 会带全量文本),不重试。

**出站映射 —— 关键设计决策**:
1. **流式状态消息从第一次发送起就必须是 interactive card，绝不能是文本：文本的 `PUT` 编辑有 20 次上限（230072）——没法用于流式；卡片 `PATCH` 无次数上限，5 QPS/消息，14 天窗口。render.rs 的节流（编辑间隔 ≥1.5s）远低于 5 QPS。`MessageRef = {"message_id","msg_type"}`；对文本 ref 调 `edit` 返回 Err → render 优雅降级。**
2. 卡片形态：schema 2.0，`config.update_multi:true`，markdown 元素固定 `element_id:"main_text"`，按钮为 `{"tag":"button","behaviors":[{"type":"callback","value":{"cb":"{nonce}:{idx}"}}]}`。markdown 里的纯文本：做转义或改用 plain_text 元素，避免 render.rs 的输出被再解释为 markdown。
3. 无按钮的普通回复：`msg_type:"text"` 并用共享的 `chunk_message` 分块（飞书文本上限 150KB ≫ 4096，共享上限是安全的）。
4. **在飞书上 answer_callback ≠ REST 调用——它就是 WS 帧 ack。** 适配器维护 pending-ack 表 `callback_query_id → oneshot<frame-ack>`；`start()` 的 card.action.trigger 处理器挂起该帧，bridge 调 `answer_callback(id, text?)` → 以 `{"toast":{...}}`（及可选的「已决」卡片）resolve；若 bridge 约 2.5s 内未响应则自动 ack `{"code":200}`（3s 硬期限）。这是与 `Platform` trait 唯一真正的阻抗失配——在适配器内部解决，不改 trait。
5. 限流器：与 Telegram 一样的按 chat token bucket（限额：p2p 5 QPS/用户、群内所有 bot 共享 5 QPS、app 1000/min）；遇到 99991400/429 与 230020 时尊重 `x-ogw-ratelimit-reset`；阻塞等待，绝不丢帧。
6. 秘密：app_secret 与 tenant_access_token 绝不出现在错误/日志里（Telegram 的 `sanitize_error` + 泄露测试模式）；token 放 Authorization header（不进 URL，比 Telegram 更省事）。

### 2d. 推迟（MVP 之后，epic 内）
- cardkit 打字机流式（`POST /cardkit/v1/cards` streaming_mode + 顺序 PUT，10 次调用/s/卡片；注意：一旦某条消息引用了 card 实体，im/v1 PATCH 会静默变成 no-op——实体卡片只能经 cardkit 更新）。
- 线程化回复 / thread_isolation、群共享 session、reaction 确认（👀/✅）、图片/文件、Lark 国际版 domain 选项（`open.larksuite.com`）、webhook 回退模式。
- 多 bot 的 `SessionKey` 消歧：目前没有任何东西区分同平台类型的两个条目（共享去重集 + SessionKey 冲突）。单个 feishu 条目没问题；若需多 app，把 app_id 折进 `platform` 字符串（`"feishu:cli_xxx"`）——比改 SessionKey 便宜。

## 3. 建议实现顺序
1. 配置字段 + 五处秘密管道 + connect.json 测试。
2. vendor 的 pbbp2 proto + WS 客户端（bootstrap/ping/ack 重组/重连）置于无 trait 的内部模块之后，用本地 WS stub 单测。
3. `FeishuPlatform` 的 REST 半边（token 缓存、reply/edit/限流器），配镜像 telegram.rs 的 wiremock 测试。
4. 注册分支 + 入站映射 + 回调的 pending-ack 桥接。
5. PR 前用真实 app 凭证做真机 spike（真机 e2e，同 tg-e2e worktree 为 Telegram 做过的那样）。

## 4. 留给维护者的开放问题
- allow_from 的 ID 类型：open_id（app 作用域，推荐）还是 user_id（租户作用域，需额外 scope）？
- MVP 群策略：仅 p2p 还是 @mention 门控的群？

2026-07-13 已决策：`domain` 配置字段随 MVP 交付（feishu/lark/自定义 https base URL，见 §2a）——维护者确认必须支持 Lark。
