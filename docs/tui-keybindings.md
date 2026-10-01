# TUI 键位

Bamboo TUI 通过唯一的动作注册表解析每一个按键。按 `F1` 可查看配置后生效的绑定；同样的解析后标签也用于标签页头部、对话框底部和命令面板。

## 加载键位映射

向任一 CLI 入口传入一个 JSON 文件：

```console
bamboo tui --keymap ./tui-keymap.json
bamboo-tui --keymap ./tui-keymap.json
```

省略 `--keymap` 时由 `BAMBOO_TUI_KEYMAP` 提供路径。该标志的优先级高于环境变量。

```json
{
  "version": 1,
  "leader": "Ctrl+\\",
  "leader_timeout_ms": 750,
  "bindings": [
    {
      "context": "global",
      "action": "show-help",
      "keys": ["F6", "Leader h"]
    },
    {
      "context": "chat",
      "action": "insert-newline",
      "keys": ["Alt+Enter"]
    },
    {
      "context": "navigation",
      "action": "switch-tab-6",
      "unbind": true
    }
  ]
}
```

一条覆盖会替换该 context/action 精确对的全部默认值。改用 `unbind: true` 代替 `keys` 即可将其移除。序列以空格分隔的按键书写，例如 `"Leader h"`；备选方案是 `keys` 中的独立条目。键名不区分大小写，支持 `Ctrl`、`Alt`、`Shift`、方向键、`Home`、`End`、`PageUp`、`PageDown`、`Tab`、`Backspace`、`Delete`、`Esc`、`Enter` 以及 `F1` 到 `F24`。

leader 超时必须在 200–5000 ms 之间。`Esc` 会取消待定序列，焦点变化也会取消它；未匹配的后续按键会报告确切的序列，而不是回落到另一个动作。如果在超时后某个输入事件在计时器竞争中获胜，它会作为新按键被重新处理，而不是被丢弃。单键的全局退出绑定总是优先于待定序列或聚焦上下文前缀；在任意位置包含同一按键的更长绑定会因不可达而被拒绝。依赖终端增强按键上报的自定义 leader 会被拒绝，以保证每个生成的 leader 回退保持可移植。

## 上下文与动作 ID

动作与上下文 ID 是稳定的 kebab-case 字符串：

- `global`：`quit-or-stop`、`show-help`、`show-notifications`、
  `open-command-palette`、`new-session`、`reopen-pending-question`、
  `open-model-picker`、`open-session-picker`、`stop-run`、
  `open-config-tab`、`open-schedules-tab`、`next-tab`、`previous-tab`
- `navigation`：`show-help`、`switch-tab-1` 到 `switch-tab-6`
- `chat`：`stop-run`、`toggle-details`、`open-slash-palette`、
  `send-message`、`insert-newline`、转录滚动以及
  `focus-conversation-blocks`
- `conversation-block`：聚焦/滚动/复制/激活动作以及 `toggle-details`
- `help`、`notifications`、`question-options`、`question-custom`、
  `question-number`、`question-inspect`：其中显示的导航、应答、检查、复制和
  取消动作；数字快捷键使用 `quick-answer-1` 到 `quick-answer-9`
- `serve-offer`、`session-delete-confirm`、`schedule-delete-confirm`：
  `confirm` 和 `reject`
- `sessions`、`mcp`、`schedules`、`schedule-form`、`skills`、`config`、
  `config-editor`：对应 F1 分组中显示的动作
- `session-picker-browse`、`session-picker-rename`、
  `session-picker-pinning`、`model-picker`、`command-palette`：各选择器
  分组中显示的动作

分发键位映射之前，请先用 F1 参考确认解析后的确切动作标签。

## 运行状态 HUD 与活动中心

Chat 底栏是一个类型化的、按 session 区分的 HUD。它把权限姿态、运行阶段与 id、连接状态、计划模式、当前工具、压缩、子代理计数和预算失败，与短暂的状态消息区分开。当工作在前台之外继续时，活动/后台 session 条会承载同样的 session 级状态。每种颜色和符号都配有文字标签。按 `Ctrl+L` 打开活动中心，持久条目在那里按 session 和运行分组；`Enter` 打开最新的关联 session。活动中心最多保留 200 条。

运行状态转换按如下方式归约：

- 本地发送：`idle|terminal -> starting`；匹配的 `execution_started`
  提供运行 id 并进入 `running`；
- token、推理、工具活动以及恢复的应答使非终态的代保持 `running`；
- 澄清与批准事件进入 `waiting for input` 和 `waiting for permission`；
  权威裁决使其回到 `running`；
- 停止请求进入 `stopping`；其响应以 `cancelled` 结束，若请求本身失败则以
  `failed` 结束；
- `complete`、`cancelled`、`error` 和 `budget_exceeded` 进入粘性终态
  阶段。该代此后重放的进度会被忽略；
- 只有此前未见过的 `execution_started` 运行 id 或显式的本地发送才能创建
  后继代。最近见过的 id 会被保留，因此延迟的启动无法重新打开更早的运行；
- SSE 就绪状态映射为 `connecting`、`online` 和 `reconnecting`；传输重试
  耗尽映射为 `offline`。重连不会重置运行状态。

工具与子代理的生命周期各自以其协议 id 为键，因此重叠的工作不会覆盖兄弟条目。它们的终态同样是粘性的，除非既有的子代对账证明了存在后继。运行明细有界保留最近 32 个工具和 64 个子代理。短暂的底栏消息在五秒后过期，不会清除任何这些持久字段。

## 校验与终端安全

完整的自定义层会原子化应用。未知字段、版本、上下文、动作或键名；重复覆盖；冲突；歧义前缀；以及不可达的必需动作都会让整个文件被拒绝。TUI 报告路径和原因，然后使用全部内置默认值——绝不会使用部分应用的映射。

首键可打印的全局序列会被拒绝，因此正常的 Chat 文本不会被捕作应用动作的开端。当多个活跃上下文的绑定共享前缀时，聚焦/模态上下文优先于更低上下文的更短绑定；兼容的更长后续按键在整个上下文栈中保持可用。自定义的 `Ctrl+S`、`Ctrl+Q` 和 `Ctrl+Z` 绑定会被拒绝，因为终端流控、复用器、SSH 或信号处理可能消费它们。`Ctrl+S`/`Ctrl+Q` 的内置兼容别名总是提供 leader 或功能键替代。`Alt+Enter` 是可移植的换行默认值；`Shift+Enter` 仍是额外的增强键盘别名。释放事件会被忽略，按住不放的按键重复不会重复触发确认、提交、删除或生命周期动作。
