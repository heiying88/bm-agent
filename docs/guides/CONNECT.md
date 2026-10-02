# 操作指南：从 IM 平台（Telegram / 飞书 / 微信）驱动 Bamboo

`bamboo-connect` 让你可以从 Telegram、飞书/Lark 或**微信个人号**与一个正在运行的 `bamboo serve` 实例对话，以此替代（或配合）HTTP API/UI——在聊天里发一条消息，它会运行一个正常的 agent 会话，回复（以及在支持可编辑消息的平台上的工具调用进度）会流式返回到同一个聊天里。

它**默认完全不启用**：没有 `connect.json`（且 `config.json` 中没有旧版 `connect` 键）时，不会启动任何后台任务。在配置至少一个平台之前，不会有任何东西监听 IM 流量。

## 1. 创建 `connect.json`

这是一个与 `config.json` **相互独立的文件**——`${data_dir}/connect.json`（同一目录，通常是 `~/.bamboo/connect.json`）。需要手动创建（目前还没有 `bamboo connect add` 这样的 CLI 动词——这是唯一仍需手工编辑的配置面）：

```json
{
  "platforms": [
    {
      "type": "telegram",
      "token": "123456789:AAH...your-bot-token...",
      "allow_from": ["987654321"]
    }
  ]
}
```

重启 `bamboo serve`（或你的 sidecar）使其生效——`connect.json` 只在启动时读取。

## 2. Telegram 设置

1. 在 Telegram 上找 [@BotFather](https://t.me/BotFather)，执行 `/newbot`，
   把它给出的 bot token 复制到上面的 `token` 里。
2. 给你的 bot 随便发一条消息，然后在服务器日志（或 Telegram 自带的
   `getUpdates` API）里找到你的数字 chat id——填进 `allow_from`。
3. **`allow_from` 为空时默认全部拒绝。** 这比 Bamboo 中其他允许列表都刻意
   更严格，因为 IM 桥接天然面向互联网（是 Telegram 的服务器主动找到你的
   bot，而不是反过来）——`allow_from` 为空或缺失时，bot 会忽略所有发送者。
4. 无需公网 IP 或 webhook：Telegram 适配器通过 HTTPS 向外部长轮询
   `getUpdates`，因此在 NAT/防火墙后面也能正常工作，就像在笔记本电脑上
   运行 `bamboo serve` 一样。

每个 Telegram 聊天对应一个 Bamboo 会话；在新聊天中发消息会开启新会话，回复以编辑/追加消息的形式流式返回，审批/澄清提示（`AgentEvent::NeedClarification` /
`ToolApprovalRequested`）在 Telegram 支持的地方渲染为内联按钮。

## 3. 飞书 / Lark 设置

飞书使用持久的 WebSocket 连接（同样不需要公网端点），并使用应用凭据而非
bot token：

```json
{
  "platforms": [
    {
      "type": "feishu",
      "app_id": "cli_a1b2c3d4e5f6",
      "app_secret": "your-app-secret",
      "domain": "feishu",
      "allow_from": ["ou_xxxxxxxxxxxxxxxxxxxxxxxxxxxx"]
    }
  ]
}
```

1. 在[飞书开放平台控制台](https://open.feishu.cn/app)（国际版产品用
   [Lark 的](https://open.larksuite.com/app)）创建一个自建应用，启用 bot
   能力，并通过“长连接”（WebSocket）传输订阅消息 + 卡片交互事件——无需
   配置公网回调 URL。
2. 把 App ID / App Secret 复制到 `app_id`/`app_secret`。
3. `domain` 选择云端：省略或填 `"feishu"` 对应 `open.feishu.cn`（中国大陆），
   `"lark"` 对应 `open.larksuite.com`（国际版），也可以为自托管/企业部署
   填一个显式的 `https://...` base URL。
4. `allow_from` 填发送者的飞书 `open_id`；与 Telegram 一样，为空时默认
   全部拒绝。

飞书会话将审批/澄清提示渲染为带按钮的交互卡片，与 Telegram 的内联键盘
体验一致。

## 4. 微信个人号设置（iLink 协议）

微信使用腾讯官方的 iLink Bot 协议（微信 8.0.70+ 内置支持）：纯 HTTP/JSON
长轮询，**无需公网 IP、无需回调 URL、无需逆向协议**——和 Telegram 一样是
Bamboo 主动外连网关（`ilinkai.weixin.qq.com`），在 NAT/防火墙后面也能正常
工作。

```json
{
  “platforms”: [
    {
      “type”: “wechat”,
      “token”: “你的 ilink bot_token”,
      “allow_from”: [“wxid_xxxxxxxx@im.wechat”]
    }
  ]
}
```

### 4.1 获取 bot_token（首次设置）

iLink 的 token 通过**扫码授权**取得，Bamboo v1 本身不提供独立的扫码命令，
用任一现成 iLink 工具完成扫码后把 token 抄进 `connect.json`：

1. 用微信官方 ClawBot 插件（`@tencent-weixin/openclaw-weixin`）或
   `cc-connect weixin setup` 等工具发起扫码登录；
2. 手机微信确认后，从该工具的配置/状态目录里复制 `bot_token`；
3. 填入上面的 `token` 字段并重启 `bamboo serve`。

### 4.2 会话过期与自动重登

iLink 的 token 有效期未公开，实测错误码 `ret=-14` 表示会话过期。适配器
遇到它会自动进入**扫码重登**：把登录二维码保存到
`~/.bamboo/connect_wechat/login_qr.png` 并在日志输出路径，你用手机扫码确认后
它会自动恢复收发。**重登拿到的新 token 只存在于进程内存中**——重启即失效，
建议扫码成功后顺手把它抄进 `connect.json` 持久化。

### 4.3 行为与已知限制

- `allow_from` 填微信用户 id（形如 `wxid_xxx@im.wechat`，可从启动后的日志
  里找到被拒绝的发送者 id）；为空时与其他平台一样**默认全部拒绝**。
- 机器人**只能回复你先发起的会话**（协议约束：回复必须携带对方消息的
  `context_token`），无法主动发消息。
- v1 支持**私聊文本 + 全格式媒体收发**：
  - **语音条**：自动使用微信自带的 ASR 转写文本（消息显示为 `[语音] 转写内容`）；
    无转写时保留占位行。
  - **图片/文件/视频（入站）**：自动从微信 CDN 下载并 AES-128-ECB 解密，保存到
    `~/.bamboo/connect_wechat/media/`，消息显示为 `[图片]/[文件]/[视频] <文件路径>`——
    Agent 可用其工具直接读取/查看。
  - **任意格式文件（出站）**：微信会话的首条消息会注入一条渠道能力说明——
    Agent 需要给你发文件时，会在回复中写 `[SEND_FILE: 文件绝对路径]` 标记行，
    网关自动经 iLink CDN（getuploadurl → AES 加密 → 上传 → sendmessage）投递，
    标记行不会显示。图片按魔数自动识别为图片消息，视频按 MP4 识别，其余按文件。
  - 审批/澄清提示整体中文化：权限提示的问题正文（引擎生成的英文模板）
    在展示层翻译成中文（如“⚠️ 需要权限确认：执行命令”），选项显示为
    “1. 允许 / 2. 拒绝”。回复数字、选项文字、中英文意图词均可命中：
    “允许/拒绝”、“跳过/忽略/skip”、“取消/算了/cancel”跨语言匹配；
    回复“都允许/不再询问”（**无论是否正有审批弹窗等待**——单独发送
    同样生效）会把**本会话切换到 Auto 审批模式**（引擎的 per-session
    机制：后续所有操作不再逐条弹出确认；危险命令等硬拒绝与计划模式
    边界仍然生效），`/new` 开新会话或回复“恢复确认”即恢复默认。批准后网关合成结构化权限回执
    （generation 绑定），被批准的工具自动重放执行——普通会话的审批在
    聊天里即可完成，无需打开 Web 端。宿主 Supervisor 会话（根编排）的
    审批例外，仍需正式决策端点处理。识别不了的回复会即时提示怎么答，
    不会静默排队。
- **只看结果**：微信渠道关闭了工具执行进度行（`⚙ Bash: …` 这类中间
  过程不推送），你只会看到最终回复、待答问题和错误提示。
- **失败不卡死**：所有会导致运行中断的内部错误（权限重放校验失败、
  状态不一致等）都会以中文错误消息推送到微信，并正常结束本轮——
  会话保持可继续对话，不会出现“选了没反应、卡住到 /new”的情况。
  - 仍推迟：群聊（`@chatroom`）、”输入中”指示、出站语音（SILK/AMR 编码）。
- 单条回复超过 2000 字符会自动分条发送；每个会话限流 1 条/秒（协议未公开
  上限，保守值）。
- 首条消息关联：启动后需要先由允许的微信账号给机器人发一条消息，会话路由
  建立后 `/new`、`/stop`、`/status` 等通道命令才可用。

## 5. 多平台 / 多 bot

`platforms` 是一个数组——想加几条就加几条，可以混用平台类型。每一条都有自己的 `id`（不设置的话会在首次保存时自动分配），并各自运行一个独立的长轮询/WebSocket 任务。

## 6. 机密

`token`（Telegram / 微信）和 `app_secret`（飞书）的静态加密方式与 Bamboo 中其他
所有机密相同——参见[配置参考](../config-reference.md#secrets-and-masking)：加载之后，对已解析配置做 `GET`/查看时，真实值会显示为 `****...****`；（通过设置 UI）原样重新提交该占位符会被视为”保留现有机密”，而不是一个新值。

## 故障排查

- **bot 完全没有响应：** 检查 `allow_from` 是否确实包含发送者的 id——空
  列表会静默丢弃所有消息（这是设计行为，不是 bug）。
- **微信 bot 突然停摆、日志出现 `ret=-14`：** iLink 会话过期。适配器会自动
  挂出扫码重登（二维码在 `~/.bamboo/connect_wechat/login_qr.png`），扫码后
  即恢复；长期方案是把新 token 写进 `connect.json`。
- **`connect.json` 似乎被忽略：** 它只在 `bamboo serve` 启动时读取；编辑
  完请重启。格式错误的 `connect.json` 会被隔离为 `connect.json.bak` 并当作
  空文件处理，而不会让服务器崩溃——在启动日志里找解析警告。
- **`config.json` 里的旧版内联 `connect` 键：** 较旧的 Bamboo 版本直接把
  这一节存在 `config.json` 里。下次加载配置时会自动迁移到 `connect.json`
  （并从 `config.json` 中剥离）——无需任何操作。
