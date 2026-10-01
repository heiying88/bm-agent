# Codex 执行器

`subagents.executor = "codex"` 支持两种传输。`codex_mode = "exec"`（向后兼容的默认值）为每次激活运行一个 `codex exec --json` 进程。`codex_mode = "app_server"` 让 `codex app-server` 常驻，并通过父级 Bamboo 审批链中继交互式命令/文件审批。

最低支持版本为 **Codex CLI 0.144.0**。在 worker 启动之前，Bamboo 会校验已安装的版本以及必需的 `exec --json`、stdin prompt、`--output-last-message`、`--config`、沙箱/危险标志和 `exec resume --json` 等能力面。实现和实时 Bamboo-provider 测试已对照 0.144.5 验证。Codex 0.144 移除了自定义 provider 的 Chat Completions 线协议，因此 `codex_wire_api` 只接受 `"responses"`。

Lotus 的子代理设置卡片暴露相同的字段，并在保存前通过 `POST /bamboo/config/validate` 校验待提交的补丁。其 Detect 按钮调用 `POST /bamboo/config/codex/detect`，可附带可选的 `{"binary":"...","mode":"app_server"}` 覆盖。响应是解析出的 `path` 和 `version`；该端点与 worker 共享同一个发现实现，因此检测成功也无法绕过 worker 的版本/能力前置检查。

## 传输契约

| 关注点 | Codex 执行器行为 |
|---|---|
| 进程生命周期 | exec 模式：每次激活一个进程组。app-server 模式：每个热 worker 一个长驻进程，逻辑线程按 Bamboo session id 隔离。 |
| prompt 传输 | assignment 写入 stdin（`-`），绝不放进 argv。 |
| 输出 | `--json --color never` 的 JSONL，外加一个有界的 `--output-last-message` 回退文件。 |
| 模型 | `codex_model` 一旦设置就作为显式的 `--model`；否则省略该标志。 |
| 仓库防护 | 真正的 Git workspace 无需覆盖。`--skip-git-repo-check` 只对 Bamboo 拥有的非 Git workspace 允许。 |
| 取消 | Bamboo 会对整棵后代进程树做快照，按最深优先的顺序发送 SIGTERM 并同时发给 Codex 进程组，然后在有界的宽限期之后把所有幸存者升级为 SIGKILL。这同样覆盖了创建新进程组的工具命令。 |

在 `app_server` 模式下，Bamboo 通过 stdio 讲按换行分隔的 JSON-RPC：`initialize`/`initialized`，然后 `thread/start` 或 `thread/resume`，再到 `turn/start`。热 worker 会保留该进程，同时一个以 session 为键的状态 map 防止一个逻辑子级继承另一个子级的线程。resume 失败会以 exec 模式同款的有界历史再水化开启一个新线程。`turn/steer` 处理来自父级的实时消息；取消操作先请求 `turn/interrupt`，若服务器未在宽限期内结束，则使用标准的进程组 TERM/KILL 阶梯。

`item/commandExecution/requestApproval` 与 `item/fileChange/requestApproval` 会被转换为 Bamboo 的 `Bash` 和 `ApplyPatch` 审批调用。bridge 缺失、中继错误和 300 秒截止时间一律应答 `decline`；审批通过则应答 `accept`。旧版的 `execCommandApproval`/`applyPatchApproval` 请求保持兼容。app-server 能力检测失败时不会静默回退到 exec 模式。

## 事件映射

| Codex JSONL 事件 | Bamboo 输出 |
|---|---|
| `thread.started` | 持久化原生线程 id，并发出 runner 元数据，包括二进制、版本、模型、认证/home 模式、沙箱、审批策略以及转发的环境变量名。 |
| `turn.started` | 第 1 轮的 `runner_progress`。 |
| `item.started/updated/completed` | agent 文本变为 token 增量；推理变为推理增量；命令/MCP/Web 条目变为 Bamboo 的工具开始/输出/完成/错误事件。 |
| `turn.completed` | 捕获输入/输出 token 用量并发出 Bamboo `complete`。 |
| `turn.failed` / `error` | 携带 Codex 消息与 stderr 尾部的有界错误结果。 |

未知的顶层或条目事件类型会被忽略而不是让运行失败，因此可以容忍 Codex schema 的增量演进。只有当必需的标志或事件契约发生变化时，Bamboo 才会提高最低版本或调整能力检查；CI 夹具覆盖已知事件，而被忽略的真机套件充当版本漂移的合并门禁。

## 认证与计费模式

`codex_auth_mode` 有四个取值。未设置时默认为 `"bamboo"`；继承个人 Codex 登录被有意设计为需要显式选择。

| 模式 | `CODEX_HOME` 与凭据 | provider 与计费 | 密钥边界 |
|---|---|---|---|
| `inherit` | 保持 `CODEX_HOME` 未设置，Codex 使用调用用户的 `~/.codex/config.toml` 和 `auth.json`。 | 用户配置的 provider；ChatGPT 登录使用该用户的订阅。 | 会继承用户的完整 Codex 配置，因此只应在确实需要这种环境性授权时使用。 |
| `api_key` | 使用 `<child-state>/codex-home`；`OPENAI_API_KEY` 必须在 `codex_forward_env` 中显式指名。 | 该 key 的 OpenAI API 计费。 | Bamboo 绝不会隐式转发 `OPENAI_API_KEY`。该 key 只存在于进程环境中。 |
| `custom` | 使用隔离的 home 和生成的 `model_providers.custom` 条目。密钥由 `codex_provider_key_ref` 选定。 | 所配置的第三方/代理 provider 及其计费策略。 | `config.toml` 只包含 `env_key = "BAMBOO_CODEX_PROVIDER_KEY"`；被引用的密钥注入到该环境变量中，绝不写入磁盘。 |
| `bamboo` | 使用隔离的 home 和生成的 `model_providers.bamboo` 条目，指向父级回环 `/openai/v1` 能力面。 | 父级 Bamboo 的 provider/路由配置；推荐默认值。 | 父级为每次激活签发一个全新的 `bcx1_` token，把它绑定到子 session 以及 Responses/models 路径，并在每条退出路径上撤销它。在 app-server 模式下，Codex 的命令认证按需从一个 Bamboo 拥有的 `0600` 文件读取该 token；该文件会在轮次结束时清空，因此长驻进程绝不会把已撤销的 token 冻结在自己的环境里。不会有任何上游 provider 密钥到达 Codex。 |

由于 `bamboo` 有意通过 `127.0.0.1` 指向父级，它要求本地 actor 部署。远程 worker 必须使用 `custom` 并提供一个从该 worker 可达的显式 provider URL；Bamboo 会在签发运行 token 之前拒绝这种有歧义的回环组合。

### 示例

推荐的父级路由模式（模式缺省时的默认值）：

```json
{
  "subagents": {
    "executor": "codex",
    "codex_auth_mode": "bamboo",
    "codex_model": "gpt-5.4"
  }
}
```

交互式父级审批中继：

```json
{
  "subagents": {
    "executor": "codex",
    "codex_mode": "app_server",
    "codex_approval_policy": "on-request",
    "codex_auth_mode": "bamboo"
  }
}
```

使用已登录用户的订阅和个人 Codex 配置：

```json
{
  "subagents": {
    "executor": "codex",
    "codex_auth_mode": "inherit"
  }
}
```

隔离的 OpenAI API key 计费需要显式的转发选择：

```json
{
  "subagents": {
    "executor": "codex",
    "codex_auth_mode": "api_key",
    "codex_forward_env": ["OPENAI_API_KEY"]
  }
}
```

自定义 provider 凭据使用既有的 Bamboo 凭据引用：

```json
{
  "subagents": {
    "executor": "codex",
    "codex_auth_mode": "custom",
    "codex_base_url": "https://provider.example/v1",
    "codex_wire_api": "responses",
    "codex_provider_key_ref": "provider.openai.api_key"
  }
}
```

`codex_base_url` 必须是不含内嵌凭据、查询参数或片段的绝对 HTTP(S) URL。它和 `codex_provider_key_ref` 只在 `custom` 模式下有效。`OPENAI_API_KEY` 只在 `api_key` 模式下才允许出现在 `codex_forward_env` 中。未知模式和线协议无法通过设置校验。

## 沙箱与审批策略

`codex exec` 没有交互式审批中继。因此 Bamboo 会在派生前解析好两个权限旋钮，绝不依赖 CLI 的隐式默认值：

| 子级姿态 | 实际的 Codex 调用 |
|---|---|
| default / restricted | `--sandbox workspace-write --config approval_policy="never"` |
| 显式配置了 Codex 只读沙箱 | `--sandbox read-only --config approval_policy="never"` |
| 父级 bypass | `--full-auto`，仍处于 workspace 沙箱内 |
| 启用 workspace 网络 | workspace-write 标志外加 `--config sandbox_workspace_write.network_access=true` |

Codex 的只读沙箱能阻止文件系统变更，但仍然开放命令执行。因此它满足不了 Bamboo 的类型化只读激活契约——后者还禁止 shell、构建、测试和可执行辅助程序。在 Codex 提供能够排除命令执行的可强制工具白名单之前，Bamboo 会在供给或分发之前拒绝 `exec` 和 `app_server` 两种模式下的类型化只读激活。Plan 的只读检视子级请使用原生 Bamboo 运行时或 Claude Code 执行器。

`codex_sandbox` 可以显式选择 `read-only`、`workspace-write` 或 `danger-full-access`。在 exec 模式下，`codex_approval_policy` 只接受 `never` 和 `on-failure`；`on-request` 会被拒绝并提示改用 app-server。在 app-server 模式下，未设置或 `on-request` 被接受，其他策略一律被拒绝，因此配置无法静默移除中继。`codex_network_access` 只作用于 workspace-write。同样的字段既可全局配置在 `subagents` 之下，也可按具名 `ExternalAgentProfile` 配置。

禁用 OS 沙箱是双重门控的。只有当子 session 的父级当前处于 bypass 模式且 `codex_allow_danger_bypass` 为 true 时，`danger-full-access` 请求才会变成 `--dangerously-bypass-approvals-and-sandbox`。否则它会降级为 `--full-auto` 并附审计警告。Root worker 总是被降级。成功的危险旁路除引导策略元数据外，还会额外发出一条醒目的警告事件。

对于非 Git 的 workspace，只有当该目录位于 Bamboo 配置的 workspace 根之下，或是携带匹配 Bamboo 生命周期所有权标记的 `<project>/.bamboo/worktree/...` 目录时，Bamboo 才会加上 `--skip-git-repo-check`。仅仅模仿那个目录形态是不够的。用户任意选择的目录会保留 Codex 的仓库检查。

## Session 身份与 resume

Codex 的转录是机器本地的。每次 `thread.started` 时，Bamboo 都会原子地把最新线程 id 写入 `<child-state>/codex-session.json`：

```json
{
  "thread_id": "...",
  "workspace": "...",
  "codex_home_mode": "isolated",
  "updated_at": "2026-..."
}
```

当 Codex 使用调用用户的 home 时 `codex_home_mode` 为 `inherit`，使用 `<child-state>/codex-home` 时为 `isolated`。持久化的 id 只有在 workspace 与 home 模式都相同的情况下才可用；任一变化都会安全回退，而不是试图去 resume 一个 Codex 看不到的转录。

激活遵循四个有界分支：

1. `RunSpec.messages` 为空表示全新激活。Bamboo 会在派生前删除过期状态，因此 rerun 绝不会意外 resume。
2. 非空 messages 加上可用的 id 会调用 `codex exec ... resume <thread_id> -`，并且只发送当前 assignment。resume 进程返回的任何新 id 都会原子地替换先前的状态。
3. 没有可用的 id 时，Bamboo 会在前面追加一段共享的、带角色标签的历史前言。保留最近约 40 条消息并控制在约 24k 字符内，更早的条目在丢弃时附上显式的截断说明，而末尾的当前用户消息会被排除，以免在 `## Current task` 下重复。
4. 如果 resume 进程在 `turn.started` 或 `turn.completed` 之前就退出，Bamboo 会清除坏 id，并以全新进程携带该回退历史恰好重试一次。轮次已有进展之后的失败以及回退尝试本身的失败都不会重试。

历史渲染器与原子 JSON 替换位于 `bamboo-subagent`，并与 `ClaudeCodeExecutor` 共享，使两个适配器的回退行为保持一致。轮中转向、多模态输入和跨机器 resume 仍不在范围内。

## 隔离与环境策略

每个子进程都在 `env_clear()` 之后启动。Bamboo 只恢复 `HOME`、`PATH`、`SHELL`、`TERM`、`LANG`、`LC_*`、`TMPDIR`、`USER` 和 `LOGNAME`，随后是经过校验的 `codex_forward_env` 名称。`CODEX_*` 变量和 `BAMBOO_CODEX_PROVIDER_KEY` 无法通过这个口子传入，因此父级/嵌套的 Codex 哨兵变量不可能泄漏进子进程，调用方也无法替换 Bamboo 管理的 provider 凭据。

对于 `api_key`、`custom` 和 `bamboo`，Bamboo 会以 `0700` 权限创建 `<child-state>/codex-home`，移除过期的 `auth.json`，并以 `0600` 权限写入一份最小的 `config.toml`。生成的 provider 配置只写环境变量名，绝不包含密钥或运行 token。执行器还会传入 `--ignore-rules`；`inherit` 则有意两者都不做，并保持 `CODEX_HOME` 未设置。

该进程拥有自己的进程组。取消操作会对其后代进程树做快照，按最深优先发送 SIGTERM 并发给整个进程组，等待五秒，再把所有幸存者升级为 SIGKILL。之所以要单独追踪后代，是因为 Codex 的工具 shell 可能创建自己的进程组；若只给进程组发信号，那个工具就会在 Codex 退出后幸存。

## 与 Claude Code 执行器的差异

| 方面 | Codex | Claude Code |
|---|---|---|
| 激活 | 每轮一个新的 `codex exec` 进程；resume 使用持久化的线程 id。 | 每次激活一个 stream-JSON 进程，配合 Claude 的 session id 与 `--resume`。 |
| 运行时审批 | v1 没有交互式审批中继；只接受 `never` 和 `on-failure`。 | 权限提示可以经 stdio 中继。 |
| 安全边界 | OS 沙箱是主要防线，解除沙箱的模式有双重门控。 | Claude 权限模式加 Bamboo 的权限中继是主要防线。 |
| 个人配置 | 显式的 `inherit` 认证模式；隔离模式会创建最小的 `CODEX_HOME`。 | 由 `claude_code_inherit_user_config` 控制。 |
| 父级 provider | 按次运行限定作用域的 Bamboo token 可以把 Codex 路由到 `/openai/v1`。 | provider 认证由 Claude CLI 登录或显式转发的环境变量名控制。 |

## Bamboo 充当 provider 的 token 与可观测性

父级 actor runner 在激活边界铸造 token，而不是在热 worker 被供给时。该 token 只经由 `RunSpec.secrets` 传递，其 debug 表示已脱敏。一个 drop guard 会在成功、出错、取消、分发失败或重试耗尽之后将其撤销。

HTTP 网关会在普通的回环旁路之前先识别 `bcx1_` 凭据。有效的 token 只能调用 `/openai/v1/responses` 和 `/openai/v1/models`；已撤销、已过期或超出作用域的凭据即使来自 `127.0.0.1` 也返回 401。被绑定的子 session 成为上游的 `LLMRequestOptions.session_id`，而正常的 OpenAI 兼容转发指标会在父级记录该请求及其结果。

## 验证

确定性的执行器与安全测试用 `cargo test` 正常运行即可。真机测试在常规 CI 中被忽略，因为它们需要已安装的外部二进制：

```sh
# 用户登录/inherit 冒烟测试与 workspace 沙箱测试
cargo test --test e2e_codex_cli_manual -- --ignored --nocapture

# 实时 Codex -> Bamboo Responses 路径、指标与运行后撤销
cargo test -p bamboo-server \
  live_bamboo_codex_completes_records_metrics_and_rejects_revoked_token \
  --lib -- --ignored --nocapture
```

这些命令覆盖的汇总检查清单如下：

1. 共享的发现实现会报告解析出的路径/版本，并在二进制缺失时给出可操作的错误。
2. 在全新的临时 Git 仓库中运行一遍会以最终文本、token 用量和引导元数据完成。
3. 第二次激活会 resume 原生线程，并回忆出一个未出现在回退历史中的 nonce。
4. `workspace-write` 允许在 workspace 内写入，并以工具错误阻止 workspace 外的写入。
5. `bamboo` 认证经由父级 `/openai/v1` 完成，记录父级指标/session 脱敏，并拒绝已撤销的作用域 token。
6. 取消会移除存活的后代进程组并返回 `Cancelled`，且同一 session 随后能成功 resume。
