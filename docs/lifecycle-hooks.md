# 生命周期钩子

Bamboo 可以在 agent 生命周期的各个事件上运行有序的命令或外部脚本处理器。在 `$BAMBOO_DATA_DIR/hooks.json`（通常是 `~/.bamboo/hooks.json`）中的 `lifecycle_hooks` 对象里配置。变更会被校验并热加载。引擎拥有的钩子会在执行开始时建立快照。通知钩子读取当前配置，后台 Bash 完成事件则读取其到达时可用的配置。

钩子匹配、进程编排和处理器执行位于独立的 `bamboo-hooks` crate 中。引擎拥有这些生命周期接缝，并应用返回的决策或上下文。

## 配置

```json
{
  "lifecycle_hooks": {
    "enabled": true,
    "PreToolUse": [
      {
        "matcher": "^Bash$",
        "hooks": [
          {
            "type": "command",
            "command": ".bamboo/hooks/audit-bash.sh",
            "timeout_ms": 5000
          },
          {
            "type": "script",
            "path": ".bamboo/hooks/check-command.js",
            "runner": "bun",
            "timeout_ms": 5000
          }
        ]
      }
    ]
  }
}
```

每个事件包含若干有序的组。组和整个区段都可以在不删除的情况下被禁用。`matcher` 是作用于工具名称的 Rust 正则表达式，仅 `PreToolUse` 和 `PostToolUse` 支持它。

处理器按配置顺序依次运行。控制决策会停止正常分发；观察者事件始终运行每一个匹配的处理器。程序化钩子与配置的处理器共享同一个分发器和优先级排序。

处理器设置：

| 类型 | 必填字段 | 可选字段 | 默认超时 |
|---|---|---|---:|
| `command` | `command` | `timeout_ms` | 60000 ms |
| `script` | `path` | `runner`, `timeout_ms` | 60000 ms |

每个 `timeout_ms` 必须介于 1 到 600000 之间。stdout 和 stderr 各自最多捕获 64 KiB。相对脚本路径以 session workspace 为基准解析。服务器拥有的钩子会回退到配置的默认工作区，然后再回退到 Bamboo 的数据目录。

`runner` 默认为 `auto`：

| 脚本扩展名 | 自动运行时顺序 | 显式 runner |
|---|---|---|
| `.js`, `.mjs`, `.cjs` | `node`，然后 `bun run` | `node` 或 `bun` |
| `.py` | `python3`，然后 `python`；Windows 还会尝试 `py -3` | `python` |
| `.sh` | 系统 sh/Bash 兼容运行时 | `bash` |
| `.ps1` | `pwsh`，然后 Windows PowerShell | `powershell` |
| `.bat`, `.cmd` | Windows 上的 `cmd.exe` | `cmd` |

Bamboo 不捆绑其中任何运行时。选定的可执行文件必须存在于为 Bamboo 子进程准备的环境中。显式 runner 必须与脚本扩展名兼容。批处理文件在共享配置中仍然有效，但在 Windows 之外运行时会报告平台诊断。

命令处理器继续通过 Bamboo 首选的 Bash 兼容 shell 运行。agent 事件使用 session workspace；服务器拥有的通知钩子会回退到配置的默认工作区，然后再回退到服务器进程目录。

支持的事件：

| 事件 | 运行时机 | 控制行为 |
|---|---|---|
| `SessionStart` | 一次运行被初始化或恢复时 | 可注入上下文或停止该运行。 |
| `UserPromptSubmit` | 在已提交的提示词被持久化之前 | 可阻塞或扩展生效的提示词。 |
| `PreToolUse` | 参数解析之后、权限检查与分发之前 | `allow`、`block` 和 `ask` 参与父 agent 的权限路径。 |
| `PostToolUse` | 前台或后台工具完成之后 | 可附加反馈；后台 Bash 完成事件使用 `tool_name: "Bash"`。 |
| `Stop` | 在运行发出终态完成之前 | 可强制一次有界的继续执行。 |
| `SessionEnd` | 终态状态已知之后 | 仅观察者；决策无法改变已定局的结果。 |
| `PreCompact` | 紧接在 LLM 上下文摘要化之前 | `additional_context` 会成为自定义摘要器指令。决策会被忽略，因为阻塞压缩有上下文溢出的风险。 |
| `Notification` | 在通知策略与去重之后，与桌面/ntfy/Bark 投递同时 | 即发即忘的观察者；决策与输出均被忽略。 |

## 输入信封

两类处理器都会通过 stdin 收到同一个带 schema 版本的 JSON 对象：

```json
{
  "schema_version": 1,
  "hook_event_name": "PreCompact",
  "session_id": "session-123",
  "workspace_path": "/work/project",
  "model": "claude-sonnet-4",
  "payload": {
    "type": "compression",
    "estimated_tokens": 170000,
    "usage_percent": 85.0,
    "max_context_tokens": 200000,
    "trigger_context_tokens": 160000,
    "trigger": "threshold",
    "phase": "pre-turn"
  },
  "timestamp": "2026-07-22T09:00:00Z"
}
```

压缩的 `trigger` 为 `threshold`、`forced_overflow_recovery` 或 `manual`。已投递的通知负载包含 `id`、`category`、`priority`、`title`、`body`、`dedup_key`、`created_at` 以及可选的 `click_url`。面向工具的信封还保留便捷字段 `tool_name`、`tool_input` 和 `tool_response` 以保持兼容。

所有处理器还会收到：

- `BAMBOO_HOOK_EVENT`：上表中的事件名称。
- `BAMBOO_SESSION_ID`：所属 session 的 id。

脚本处理器额外收到 `BAMBOO_HOOK_SCRIPT`，即解析后的脚本路径。

## 输出契约

对于具备决策能力的事件，处理器可以向 stdout 写入一个响应对象：

```json
{
  "decision": "block",
  "reason": "production deletion is forbidden",
  "additional_context": "Use the staging workspace instead."
}
```

`decision` 为 `allow`、`block` 或 `ask`。`additional_context` 可以随决策一起返回，也可以不带决策单独返回。

处理器以退出码 0 结束，并写入空 stdout 或恰好一个 JSON 响应。退出码 2 表示阻塞，原因取自 stderr。其他非零退出、格式错误或被截断的 stdout、缺失的运行时以及超时都会被记录并视为非阻塞失败。dry-run 端点会返回这些诊断信息，而不持久化配置。

观察者事件（`SessionEnd`、`Notification`）从不改变控制流。`PreCompact` 只消费 `additional_context`；其决策与退出码 2 的阻塞信号会被有意忽略。

## 脚本示例

Node.js 或 Bun（`.bamboo/hooks/check-command.js`）：

```javascript
let raw = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => (raw += chunk));
process.stdin.on("end", () => {
  const input = JSON.parse(raw);
  const command = input.tool_input?.command ?? "";
  const response = command.includes("rm -rf /")
    ? { decision: "block", reason: "root deletion is forbidden" }
    : {};
  process.stdout.write(JSON.stringify(response));
});
```

Python（`.bamboo/hooks/add-context.py`）：

```python
import json
import sys

payload = json.load(sys.stdin)
print(json.dumps({
    "additional_context": f"hooked {payload['hook_event_name']}"
}))
```

Shell（`.bamboo/hooks/block-dangerous-bash.sh`）：

```bash
#!/usr/bin/env bash
set -euo pipefail
payload=$(cat)
command=$(printf '%s' "$payload" | jq -r '.tool_input.command // ""')
if printf '%s' "$command" | grep -Eq '(^|[;&|[:space:]])rm[[:space:]]+-rf[[:space:]]+(/|~)'; then
  printf '%s\n' 'dangerous recursive deletion is blocked' >&2
  exit 2
fi
```

## 进程与安全模型

每次脚本调用都是一个全新的子进程。Bamboo 提供准备好的环境和工作目录，把信封写入 stdin，并发地抽取有界的 stdout/stderr，强制执行配置的挂钟期限，并在超时时杀死进程树。

外部脚本不受沙箱保护。它们以 Bamboo 进程用户的文件系统、网络、环境和操作系统权限运行。只应启用可信的、用户自有的钩子配置和脚本。项目本地钩子的发现需要一个单独的信任门禁。需要更强隔离时，请使用操作系统/容器沙箱或受限的服务账户。

编辑后自动格式化：

```json
{
  "PostToolUse": [{
    "matcher": "^(Write|Edit)$",
    "hooks": [{"type": "command", "command": "cargo fmt", "timeout_ms": 30000}]
  }]
}
```

生命周期钩子 dry-run 端点接受两种处理器类型，并使用生产级的输入 schema、超时、输出上限、工作目录、运行时选择，以及为所选事件生成的确定性合成负载。
