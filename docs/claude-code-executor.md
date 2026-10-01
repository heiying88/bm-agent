# 将 Claude Code 作为外部 agent 驱动——协议参考

状态：**已实现**——`ClaudeCodeExecutor` 位于 `src/claude_code_executor.rs`，通过 `subagents.executor = "claude_code"` 选用（面向用户的配置面参见[配置参考](config-reference.md#sub-agents--external-cli-executors)）。本文档保留为协议参考细节，供需要改动/扩展该执行器的人查阅——它是最初的设计记录。来源：提炼自 `chenhg5/cc-connect` 的 `agent/claudecode/`（Go 实现，在生产环境中经受过 Claude Code 2.x 的实战检验），并对照该仓库 `main@2026-07-11` 提交验证过。下文的文件引用均指向该仓库，行号未必与当前 bamboo 实现完全一致。

## 它在 bamboo 中的接入点

`bamboo-subagent::provision::ExecutorSpec` 预留了该槽位（`provision.rs:221`）：

```rust
/// 将外部 CLI agent 包装为引擎。
CliAdapter { command: String, args: Vec<String> },
```

最初的实现计划（留档）：

1. `ClaudeCodeExecutor: ChildExecutor`（`executor.rs:169`）——持有子进程，负责 `RunSpec` → stream-json stdin、stream-json stdout → `EventSink`、`SteerInbox` → 轮中用户注入/权限响应、`CancellationToken` → 优雅关停（见 §7）的转换。
2. worker 执行器工厂：把 `ExecutorSpec::CliAdapter`（或携带 model/permission-mode/resume-id 的专用 `ClaudeCode` 变体）映射到它。
3. `bamboo-engine/src/external_agents/runtime.rs:104,215`——在接受 `"echo"` / `"bamboo_runtime"` 的同时接受执行器种类 `"claude_code"`。

## 1. 派生

**每个 session 一个长驻进程**（而不是每条消息一个）。重复的轮次就是向同一进程重复写入 stdin；`--resume` 仅用于进程死亡后的重新挂接。

```
claude \
  --output-format stream-json \
  --input-format  stream-json \
  --permission-prompt-tool stdio \
  --replay-user-messages \
  --verbose \
  --permission-mode <acceptEdits|plan|bypassPermissions|default>  # 总是显式传递——见下文
  [--strict-mcp-config] [--setting-sources project|""]            # 隔离——见下文
  [--resume <session_id>]                                    # 全新 session 时省略
  [--model <model>]
  [--tools Read,Glob,Grep] [--disallowedTools Bash Edit ...]
  [--system-prompt <s>] [--append-system-prompt-file <path>]
```

（cc-connect：`agent/claudecode/session.go:234-322`）

**`--permission-mode` 总是被显式传递（issue #443，关键）。**针对 claude 2.1.207 的真机 e2e 测试发现，完全省略该标志时，无头 stream-json 的默认值是 `auto`——它会自动批准每一个工具，且从不发出 `can_use_tool` 询问。因此 `ClaudeCodeExecutor::build_command` 在已配置时总是发送 `permission_mode`，否则发送字面字符串 `default`——绝不什么都不发。正是这一点让 §3 中"没有 host bridge → 除非 `bypassPermissions` 否则拒绝"的本地决策策略真正得以触发；在修复之前它是不可达的死代码（每个询问都被 CLI 自己自动批准，因此执行器本会拒绝的任何操作都不会触发 `control_request`）。

**默认与调用方用户的 `~/.claude` 隔离（issue #443）。**同一场 e2e 测试显示，子进程加载了用户的全部全局配置：6 个 MCP 服务器（其中包括一个桌面控制服务器）、所有已安装的 skill 以及 memory 路径——为一次 `touch` 付出约 8k 缓存创建 token 和一大片环境性授权面。除非在 `ClaudeCode` 执行器 spec 上设置 `inherit_user_config: true`，`build_command` 都会加上 `--strict-mcp-config` 和 `--setting-sources project`，使子进程只看到项目作用域的配置，而不是用户的全局配置。类型化的只读激活即使被请求继承，也会额外传一个空的 `--setting-sources` 值。这可以把仓库控制的设置、CLAUDE.md、skill，尤其是 hook 挡在进程之外：Claude 的 `plan` 权限模式能约束内置工具，却不能沙箱化 hook 子进程。

环境（issue #443——环境变量白名单取代了先前逐个剔除变量的做法）：
- 子进程在 `env_clear()` **再加一份显式白名单**之下派生：`HOME`、`PATH`、`SHELL`、`TERM`、`LANG`、`LC_*`（前缀）、`TMPDIR`、`USER`、`LOGNAME`。父进程环境中的其他一切——包括任何 `*_API_KEY`——都在构造层面被剥离，而不是靠黑名单。
- 执行器 spec 上的 `forward_env: Vec<String>` 指定要在白名单之外额外逐字转发的变量。以这种方式转发 `ANTHROPIC_API_KEY` 是一次显式选择，会把计费从 CLI 自身的订阅认证切到 API key——见 §6。
- `CLAUDECODE` 在白名单处理之后仍会被显式 `env_remove`——既然有了 `env_clear()`，它本来就不可能从父进程泄漏进来，这一步属于冗余，保留它是作为下文嵌套 session 风险的可执行文档。
- worker 会把宿主下发的 `disabled_tools` 转发为裸的 `--disallowedTools` 规则。类型化的只读激活还会把正向内置工具面设置为 `--tools Read,Glob,Grep` 并拒绝 `mcp__*`。这样即使 Claude 原生的 `plan` 模式作为又一层防线仍然开启，Bash、变更、委派、网络和新加入的内置工具也都被挡在子进程之外。
- 父级在向本地 worker 发送任何类型化只读供给之前，会先运行 `subagent-worker --print-capabilities` 并要求显式的 `typed_read_only_tool_policy_v1` 确认。这发生在 assignment 或 provision 文档送达之前。变更前的 Bamboo worker 会拒绝未知标志，缺少该确认的自定义 worker 会被拒收，因此前向兼容的 JSON 字段跳过不可能静默抹掉只读边界。
- 把子进程放进它自己的**进程组**，这样关停时可以杀掉整棵进程树（claude → 它的 MCP 服务器）（`session.go:369`）。

嵌套 session 风险：Claude Code 会检测自己的 `CLAUDECODE` 环境变量，一旦从外层 session 继承到它就会行为异常（`session.go:372`）。

注意事项：
- 经 claude-code-router 路由时去掉 `--verbose`——路由器输出会破坏 JSON 流（`claudecode.go:524-526`）。
- euid 0 下的 `bypassPermissions` 会被 CLI 拒绝；降级并给出警告（`session.go:225-229`）。
- 即使在 `default` 模式下，CLI 自带的沙箱也会不询问就自动运行普通的只读命令（例如一个裸 `echo`）——要验证权限中继，需要一条有真实副作用（写文件）的命令。

## 2. 线上协议

双向均为按换行分隔的 JSON。**读取方必须允许 10 MB 的单行**（`session.go:472` 使用 10 MB 的扫描缓冲；工具结果可能非常大）。

### stdin → claude（用户轮次）

```json
{"type":"user","message":{"role":"user","content":"fix the failing test"}}
```

多模态内容使用 parts：

```json
{"type":"user","message":{"role":"user","content":[
  {"type":"text","text":"what is in this screenshot?"},
  {"type":"image","source":{"type":"base64","media_type":"image/png","data":"..."}}
]}}
```

非图片文件不做内联：把它们写入一个临时目录，并在 prompt 文本中引用其绝对路径（"文件已保存在本地，请读取：……"）——agent 会用它自己的 Read 工具打开它们（`core/message.go:103-141`）。

### stdout → 执行器，按顶层 `type` 分发

| type | 含义 | 需提取的内容 |
|---|---|---|
| `system` | session 引导 | `session_id`（由 agent 分配——持久化以备 resume）、`model` |
| `assistant` | 一条模型消息 | 遍历 `message.content[]`：`text` → token/文本事件；`thinking` → thinking 事件；`tool_use` → 工具开始事件（`name`、`input`）。`message.usage` 给出实时上下文数字（其 `output_tokens` 是占位值，忽略） |
| `user` | 回显的工具结果 | `content[].type == "tool_result"` → 工具结束事件（`content` 被截断、`is_error`） |
| `result` | 轮次结束 | 最终文本、`session_id`、token 总量。**`subtype: "compact"/"compaction"` 是轮中事件**——不要当作轮次完成（cc-connect issue #481） |
| `control_request` | 权限询问 | 见 §3 |
| `control_cancel_request` | CLI 撤回了一个待处理的询问 | 丢弃对应的待处理审批 |

未知类型：以 debug 级别记录日志，绝不让流失败（`session.go:587-594`）。进程退出时，把 stderr 作为错误事件抛出，并恰好完成一次运行（`session.go:512-535`）。

## 3. 权限中继（`--permission-prompt-tool stdio`）

CLI 在每次受门控的工具调用前发起询问：

```json
{"type":"control_request","request_id":"r1","request":{
  "subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm -rf build"}}}
```

执行器在本地决策（对 `bypassPermissions` 等价模式自动放行，对 acceptEdits 自动放行编辑类工具等），或者以 NeedsHuman 风格的事件中继给父级，然后通过 stdin 应答：

```json
{"type":"control_response","response":{
  "subtype":"success","request_id":"r1",
  "response":{"behavior":"allow","updatedInput":{"command":"rm -rf build"}}}}
```

拒绝：`{"behavior":"deny","message":"user denied"}`。放行时，把工具 `input` 原样回填为 `updatedInput`（也可以被编辑）。`AskUserQuestion` 的 control_request 携带结构化问题——应映射到 bamboo 的 QuestionDialog 路径而非权限路径（`session.go:856-937`）。

用 bamboo 的术语说：发出携带 `request_id` 的 `NeedsHuman` 事件，把待处理审批挂在一个由 oneshot channel 组成的 map 中，当 steer inbox 送达决策时再解决它——与 cc-connect 中 codex app-server 适配器的做法一致（`appserver_session.go:542-617`）。

**中继超时（issue #443）。**当确实挂接了 host bridge 时，`decide_and_respond` 会把 `HostBridge::approval_call` 包在以 `APPROVAL_RELAY_TIMEOUT`（300 秒）为上限的 `tokio::time::timeout` 里。永不应答的宿主审批者（UI 崩溃、孤儿 session）不会再把 CLI 轮次永久挂起——到期时执行器以 `"approval relay timed out after 300s; denying"` 拒绝，轮次继续。这与 `approval_call` 既有的错误路径（应答 `oneshot` 发送端被丢弃）不同，后者仍按立即拒绝处理。

## 4. Session 身份与 resume

已在 `ClaudeCodeExecutor` 中实现（issue #444）。`RunSpec.messages` 是激活判别式（`proto.rs:28`）：actor 首次激活时为空，重新激活（`send_message`/`update`/`rerun`）时非空，后者会带上该 actor 之前的会话。

**持久状态。**执行器把 agent 分配的 session id 持久化在子进程稳定的按激活存储目录中——其解析方式与 `BambooRuntimeExecutor::build`（`subagent_worker.rs:194-202`）完全一致：父级已隔离时用 `spec.storage_dir`，否则用 `$TMPDIR/bamboo-subagents/<child_id>`。两个 worker 工厂分支（`subagent_worker.rs`、`broker_agent.rs`）都会解析该目录并传入 `ClaudeCodeExecutor::new` 的 `state_dir` 参数。

状态文件：`<dir>/claude-code-session.json`

```json
{ "session_id": "...", "workspace": "...", "updated_at": "2026-..." }
```

每一帧携带 `session_id` 的 `system` 或 `result` 都会触发原子写入（同目录下的临时文件 + `rename`）——resume 后的 session 可能被分配一个全新的 id，因此这里总是重新捕获，而不是假设它稳定。`workspace` 会与 id 一并记录：Claude Code 的转录是机器本地的，位于 `~/.claude/projects/<hashed-workdir>/` 之下，因此之后针对**另一个** workspace 的激活（不同项目，或干脆是另一台机器）会把持久化的 id 视为不可用。

**激活逻辑**（`ClaudeCodeExecutor::run`）：

1. `messages` 为空 → 全新 session；先删除任何过期的状态文件（`rerun` 绝不能意外 resume）。
2. `messages` 非空且状态文件记录了针对**同一** `workspace` 的 id → 带 `--resume <id>` 派生，只发送当前 assignment（CLI 已经拥有转录）。
3. `messages` 非空但没有可用的 id（本机首次运行、存储被 GC、workspace 变更）→ **回退再水化**：把携带的历史渲染成一段有界的文本前言（带角色标签，`**role**: content`，最多保留最近约 40 条消息/约 24k 字符，最旧的先丢弃，并附上显式的 `_[truncated: N earlier message(s) omitted]_` 说明），用 `## Prior conversation (rehydrated)` / `## Current task` 标题清晰分隔，使模型不会把再水化的上下文与当前任务混淆，然后把它置于 assignment 之前。assignment 自身的末尾用户消息（按线上契约随 `messages` 携带）会被排除在前言之外，避免重复。同时记录一条警告；上下文绝不会被静默丢弃。
4. **resume 失败重试：**如果带 `--resume` 的派生在发出终止 `result` 帧之前就退出（session id 无效/被 GC——CLI 会很快报错），则在清除过期状态文件后，使用与第 3 步相同的回退再水化，不带 `--resume` 重试**一次**。除了这一次尝试外没有重试循环，而且该重试只针对这一特定失败模式（一次从未产出 result 的 `--resume` 尝试）——其他错误一律原样返回。

**（仍然）不做的事。**轮中转向到同一 session 上真正的新轮次，以及多模态——不受本次变更影响。环境变量转发已随 #443 发布（见 §1/§6）。跨机器 resume 在构造上就不在范围内（见上文 workspace/机器局部性说明）；如果 actor 被重新部署到另一台主机或另一个 workspace，激活会自动落入回退再水化。

## 5. 取消/关停

优雅的三阶段关闭（`session.go:1171-1228`）：

1. 关闭 stdin（EOF 让 CLI 执行其 Stop hook），
2. 最多等待约 120 秒退出，
3. 向进程组发送 SIGTERM，等待 5 秒，再对整组 SIGKILL。

在该协议上没有可靠的轮中中断手段（cc-connect 的 `/stop` 是杀掉进程后按 id resume）。把 `CancellationToken` 映射为完整关闭；延续依赖 `--resume`。

## 6. 计费说明

派生出的二进制是官方 Claude Code，使用用户自己的登录；截至 2026-07，订阅认证仍覆盖 `claude -p`/stream-json 用量（6 月 15 日的积分拆分被暂停了）。真机 e2e 证实了这一点：`system.init` 帧报告 `apiKeySource: "none"`，子进程环境中没有任何 key——真正被计费的是订阅。

**环境策略（issue #443，已实现）。**执行器不再整体转发父进程环境。`build_command` 让子进程运行在 `env_clear()` 加固定白名单之下——`HOME`、`PATH`、`SHELL`、`TERM`、`LANG`、`LC_*`（前缀）、`TMPDIR`、`USER`、`LOGNAME`——这足以让 CLI 及其 shell 工具正常工作，同时在构造层面排除所有 `*_API_KEY` 和其他环境性密钥。执行器 spec 上的 `forward_env: Vec<String>`（以及对应的 `claude_code_forward_env` 配置字段，§8）指定要额外逐字转发的变量；以这种方式转发 `ANTHROPIC_API_KEY` 是一次**显式**选择，会把计费从订阅切到 API key——绝不是隐式默认。

## 7. 执行器形态（按当前实现）

```rust
pub struct ClaudeCodeExecutor {
    binary: String,
    model: Option<String>,
    permission_mode: Option<String>,
    workspace: Option<String>,
    /// 存放 resume session 状态文件的按子进程稳定目录——见 §4。
    /// `None` 表示禁用 resume 持久化（每次激活都是全新的）。
    state_dir: Option<PathBuf>,
    /// Issue #443：`false`（默认）会加上 `--strict-mcp-config` +
    /// `--setting-sources project`；只读 Plan 使用空的来源列表。
    inherit_user_config: bool,
    /// Issue #443：在固定白名单之外额外转发的环境变量
    /// 名称——见 §1/§6。
    forward_env: Vec<String>,
    /// Issue #443：权限中继 `HostBridge::approval_call` 的上限
    /// ——见 §3。测试之外恒为 `APPROVAL_RELAY_TIMEOUT`（300 秒）。
    relay_timeout: Duration,
}

#[async_trait]
impl ChildExecutor for ClaudeCodeExecutor {
    async fn run(&self, spec: RunSpec, events: EventSink,
                 steer: SteerInbox, cancel: CancellationToken) -> ChildOutcome {
        // steer 会在整个激活期间（包括下面可能的两次尝试）被排空，
        // 但不会被响应——没有可靠的轮中中断（§5）。
        //
        // §4 激活逻辑，然后调用 `run_once`（spawn → select-loop →
        // §5 关停）一次，或在 resume 失败重试时调用两次：
        // 1. messages 为空        → 删除过期状态；全新 spawn。
        // 2. messages 非空且有 id → 带 `--resume <id>` spawn，只发送
        //                            当前 assignment。
        // 3. messages 非空、无 id → 回退：渲染的历史前言 + assignment，
        //                            全新 spawn。
        // 4. 未产生 `result` 帧就退出的 `--resume` spawn → 清除
        //    状态，用回退内容重试一次，不带 `--resume`。
        //
        // 在每次 `run_once` 尝试内部，select-loop：
        //    - stdout 行   → 解析 → events.emit(...)   （§2 表格）
        //      带 session_id 的 `system`/`result` → 持久化状态（§4）
        //      control_request → events.emit(NeedsHuman) + 挂起 oneshot
        //    - cancel      → §5 关停 → ChildOutcome::Cancelled
        //    - `result`    → §5 关停 → ChildOutcome::Completed
    }
}
```

## 8. 配置管道（issue #443）

`ExecutorSpec::ClaudeCode`（`bamboo-subagent::provision`）携带 `binary`、`model`、`permission_mode`、`inherit_user_config: Option<bool>` 和 `forward_env: Option<Vec<String>>`。把 spec 变成运行中 `ClaudeCodeExecutor` 的两个工厂分支（`src/subagent_worker.rs`、`src/broker_agent.rs`）都会把为 `None` 的隔离/环境字段解析为加固后的默认值（`inherit_user_config.unwrap_or(false)`、`forward_env.unwrap_or_default()`）。

有两个配置入口可以基于 `executor = "claude_code"` 构造 `ClaudeCode` spec：

- `bamboo_config::SubagentsConfig`——内置的本地 actor worker（`subagents.claude_code_binary` / `_model` / `_permission_mode` / `_inherit_user_config` / `_forward_env`）。
- `bamboo_engine::external_agents::config::ExternalAgentProfile`——使用 actor 协议的具名 `externalAgents` profile（相同的 `claude_code_*` 字段名）。

两者分别在 `crates/engine/bamboo-engine/src/external_agents/runtime.rs` 中被解析为 spec（对应 `build_local_actor_runner` 与 `build_external_child_runner`）。
