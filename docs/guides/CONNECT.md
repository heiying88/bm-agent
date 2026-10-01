# 操作指南：从 IM 平台（Telegram / 飞书）驱动 Bamboo

`bamboo-connect` 让你可以从 Telegram 或飞书/Lark 与一个正在运行的 `bamboo serve` 实例对话，以此替代（或配合）HTTP API/UI——在聊天里发一条消息，它会运行一个正常的 agent 会话，回复（以及在支持可编辑消息的平台上的工具调用进度）会流式返回到同一个聊天里。

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

## 4. 多平台 / 多 bot

`platforms` 是一个数组——想加几条就加几条，可以混用平台类型。每一条都有自己的 `id`（不设置的话会在首次保存时自动分配），并各自运行一个独立的长轮询/WebSocket 任务。

## 5. 机密

`token`（Telegram）和 `app_secret`（飞书）的静态加密方式与 Bamboo 中其他
所有机密相同——参见[配置参考](../config-reference.md#secrets-and-masking)：加载之后，对已解析配置做 `GET`/查看时，真实值会显示为 `****...****`；（通过设置 UI）原样重新提交该占位符会被视为“保留现有机密”，而不是一个新值。

## 故障排查

- **bot 完全没有响应：** 检查 `allow_from` 是否确实包含发送者的 id——空
  列表会静默丢弃所有消息（这是设计行为，不是 bug）。
- **`connect.json` 似乎被忽略：** 它只在 `bamboo serve` 启动时读取；编辑
  完请重启。格式错误的 `connect.json` 会被隔离为 `connect.json.bak` 并当作
  空文件处理，而不会让服务器崩溃——在启动日志里找解析警告。
- **`config.json` 里的旧版内联 `connect` 键：** 较旧的 Bamboo 版本直接把
  这一节存在 `config.json` 里。下次加载配置时会自动迁移到 `connect.json`
  （并从 `config.json` 中剥离）——无需任何操作。
