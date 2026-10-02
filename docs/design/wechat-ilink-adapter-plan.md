# 微信个人号适配器方案（iLink 协议）—— 已选定的首个微信通道

> 状态：**已批准实施**（2026-10-01 用户拍板：先接官方 iLink，企业微信与 OpenClaw 桥接后续再议）。
> 各组件的关系与数据流讲解见 [`wechat-ilink-architecture.md`](wechat-ilink-architecture.md)。
> 协议参考：腾讯 iLink Bot API（微信 8.0.70+ 内置），逆向还原文档见
> [cc-weixin/weixin-bot-api.md](https://github.com/hao-ji-xing/cc-weixin/blob/main/weixin-bot-api.md)，
> 实践参考 [cc-connect 微信接入指南](https://github.com/chenhg5/cc-connect/blob/main/docs/weixin.md)。

## 1. 协议要点（实现依据）

| 维度 | 事实 |
|---|---|
| 传输 | 纯 HTTP/JSON，`https://ilinkai.weixin.qq.com`（登录响应还会返回每 bot 独立的 `baseurl`）；**无回调、无 WebSocket、无需公网 IP** |
| 认证 | 每个请求带三个头：`Authorization: Bearer <bot_token>`、`AuthorizationType: ilink_bot_token`、`X-WECHAT-UIN`（随机 uint32 的十进制字符串再 base64，防重放，**每次请求新生成**） |
| 收消息 | `POST /ilink/bot/getupdates` 长轮询（服务端挂起约 35 秒）；请求体回传上次响应的 `get_updates_buf` 游标——**游标必须持久化，否则消息重复投递** |
| 发消息 | `POST /ilink/bot/sendmessage`；请求体的 `msg.context_token` 必须**原样回传**触发消息的 context_token，机器人只能回复用户先发起的会话 |
| 消息形态 | `msgs[]`：`from_user_id`（`@im.wechat` 结尾）、`to_user_id`（`@im.bot`）、`message_type`（1=入站 / **2=机器人出站回显，必须过滤否则自回复死循环**）、`item_list[].type`（1=文本，2=图片，3=语音含转写，4=文件，5=视频） |
| 会话过期 | 错误码 `ret == -14`：需重新扫码换 token；未公布 token 有效期 |
| 媒体 | 走 CDN `novac2c.cdn.weixin.qq.com` + AES-128-ECB 加解密（**v1 不做**） |
| 其他端点 | `get_bot_qrcode`（bot_type=3）、`get_qrcode_status`（轮询扫码状态，confirmed 时返回 `bot_token`+`baseurl`）、`getconfig`/`sendtyping`（输入中指示，v1 不做）、`getuploadurl`（媒体，v1 不做） |
| 未公开 | 单条消息长度上限、频率限制、错误码表、context_token 有效期——均以保守值起步、真机 spike 校准 |

## 2. 设计决策

### 2a. 配置（零 bamboo-config 改动）

复用 `ConnectPlatformConfig` 既有字段，语义映射：

```json
{
  "type": "wechat",
  "token": "ilink bot_token（Bearer，走既有加密管道）",
  "domain": "https://ilinkai.weixin.qq.com（可选，默认即此值；仅接受 https:// 前缀）",
  "allow_from": ["wxid_xxx@im.wechat"]
}
```

- `token` 复用 telegram 同款加密落盘/凭据库/掩码管道（`token_encrypted` 等），**秘密管道五处往返全部免改**；
- `domain` 复用飞书的"可选自定义 base URL"语义，新增 `resolve_wechat_base_url`（默认官方域；`https://` 前缀自定义；其他值无效跳过）；
- `allow_from` 精确匹配 `from_user_id`，空列表 = 全拒（bridge 既有语义，含启动 warn）。

### 2b. 注册（照抄 telegram 臂的三个接缝）

`ConnectManager::start` 加 `"wechat"` 分支：token 为空 → warn + skip（与 telegram/feishu 同约定：**显式凭据，不猜**）；`multi_bot_guard` 与 `platform_config_will_start` 各加 `"wechat" => token 非空 && domain 合法` 谓词；`platforms/mod.rs` 加 `pub mod wechat;`。

### 2c. 适配器本体（单文件 `connect/platforms/wechat.rs`，与 telegram 同构）

- **Capabilities 全 false**（纯文本、无按钮、无编辑、无媒体）→ render 自动走 legacy 分条模式；审批走编号文本列表（`match_text_answer` 中文关键词开箱即用）；`answer_callback` 用 trait 默认 no-op。
- **字段映射**：`platform="wechat"`、`chat_id = from_user_id`（v1 仅私聊）、`user_id = from_user_id`、`sent_at = create_time（若上游补充）否则 Utc::now()`（协议未公开时间戳字段，重启积压的过期丢弃因此弱化——靠游标持久化兜底）、`reply_ctx = { to_user_id, context_token }`（原样回传给 `reply`）。
- **message_id 合成**：协议无消息 id；用 `DefaultHasher(from_user_id + context_token + 文本 + 批内序号)` 的确定性哈希——同一消息重放时哈希一致，bridge 去重键 `wechat:<hash>` 才能命中；正常路径无碰撞风险。
- **游标持久化**：`{data_dir}/connect_wechat/cursor.json`（非秘密，原子写）；构造时加载、每次成功 getupdates 后保存。`state_dir` 由注册臂从 `ConnectManager::start` 已有的 `data_dir` 参数传入（`connect_wechat/` 子目录）。
- **分块**：`render::chunk_message(text, 2000)`——微信未公开上限，取 2000 字符保守值（常量 `WECHAT_MESSAGE_CHARS`，spike 后校准）。
- **限流**：照抄 telegram 的每 chat 1 msg/s `RateLimiter`（含过期条目清扫），键 = `to_user_id`。
- **错误脱敏**：token 在 `Authorization` 头而非 URL，仍按 telegram 同款 belt-and-braces：`without_url()` + 字面量替换，配秘密泄露单测。
- **-14 会话过期的自愈**：进入扫码重登循环——`get_bot_qrcode` 把二维码 PNG 写到 `state_dir/login_qr.png` 并在日志输出路径与登录链接，轮询 `get_qrcode_status` 至 confirmed（超时 480 秒），成功后用新 token 恢复轮询并 warn 提示把它写入 connect.json；超时则退避 5 分钟后再次挂出二维码。
- **首启 token 获取**（v1 约定）：用任一 iLink 工具（微信官方 ClawBot 插件 / openclaw-weixin / cc-connect `weixin setup`）扫码取得 bot_token 后填入 connect.json。后续可选做 `bamboo connect wechat login` CLI 子命令（列入推迟项）。

### 2d. 推迟项

- ~~入站媒体~~（已实现：语音转写 + 图片/文件/视频 CDN 下载/AES-128-ECB 解密/落盘）；
- ~~出站任意格式文件~~（已实现，纯网关层方案：`[SEND_FILE: 路径]` 回复标记约定 +
  bridge 首条消息注入说明 + 适配器解析标记并经 getuploadurl→AES 加密→CDN 上传→
  sendmessage 投递；引擎与共享 OutboundMessage 契约零改动）；
- 输入中指示（getconfig/sendtyping）；
- 群聊（`@chatroom` id 路由 + 群内发送者识别，协议字段未公开，需 spike）；
- 出站语音（SILK/AMR 编码，cc-connect 亦仅支持 AMR 转码）；
- `bamboo connect wechat login` 独立 CLI；多账号（`wechat:<bot_id>` 折叠键，同 feishu 计划 §2d）；
- 同一 context_token 多次回复的合法性未知（legacy 渲染一次 run 会发多条）——spike 项；若被拒，缓解方案是适配器内做短窗口合并。

## 3. 测试（wiremock，镜像 telegram.rs 的桩模式）

1. 入站映射：文本消息 → InboundMessage 字段/reply_ctx；`message_type=2` 出站回显被过滤；非文本 item 跳过。
2. 游标：首请求 `get_updates_buf` 为空串，回传上次响应值，并落盘 cursor.json。
3. 出站：reply 原样回传 context_token、超长文本按 2000 字符分块、逐块限流。
4. `ret=-14` → 错误文本包含 -14（进入重登路径的判据）。
5. 传输错误不泄露 bot token。
6. 扫码重登：桩掉 get_bot_qrcode/get_qrcode_status，-14 后自动重登成功恢复。
7. 注册谓词：multi_bot_guard / platform_config_will_start / resolve_wechat_base_url 的 wechat 用例（镜像 mod.rs 现有测试）。

## 4. 文档交付

- `docs/guides/CONNECT.md` 增加微信小节（配置示例、token 获取步骤、allow_from 语义、扫码重登行为、已知限制）；
- 本设计文档随实施更新状态；`docs/README.md` 索引同步。

## 5. 实施顺序

1. `wechat.rs` 适配器 + 全部单测；2. 三个注册接缝 + 谓词测试；3. `cargo check`/`cargo test -p bamboo-server connect` 通过；4. CONNECT.md 与索引；5. 真机 spike（用户侧：扫码拿 token → 配 connect.json → 微信发消息验收 `/new` `/stop` 与编号审批）。
