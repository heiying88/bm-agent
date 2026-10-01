# 易用性 SDK 规划：将 `SubagentProfile` 提升进 `bamboo_agent::agent`

**状态：**设计 / 实现规格
**作者：**首席架构师（综合 6 份 explorer 报告 + 直接重读源码）
**日期：**2026-06-04

> **评审后更新：**根门面（`bamboo_agent::agent`）是**基于 instruction 的，而不是
> 基于 profile 的**。profile 语法糖方法（`.researcher()` / `.coder()` /
> `.from_profile()` 等）以及门面的 `profiles` re-export 均已移除。门面现在是
> `Agent::builder().model(..).instruction(..).tools(..)`——由调用方提供自己的
> system-prompt 片段，引擎在运行时组装完整提示词。`SubagentProfile` 体系、
> `ProfileRunner` 以及 profile 迁移（阶段 2–3）仍保留在 `bamboo-engine` 中，服务于
> sub-agent / Task 子系统。

## 0. 目标与指导约束

把现有的 `SubagentProfile` 机制提升为一个暴露在 `bamboo_agent::agent` 的一等、易用的 SDK，让库使用者可以这样写：

```rust
let agent = Agent::builder().researcher().model("...").build()?;
agent.run(&mut session, "investigate X").await?;
// 或者，profile 驱动的子 spawn：
runner.run_profile(profile, input).await?;
```

### 硬约束（与任何 explorer 建议冲突时，以这些约束为准）

1. **依赖方向神圣不可侵犯：**`domain ← tools ← engine ← root facade`，
   `infrastructure` 与 `agent-core` 作为共享底层。**不允许反向边。**已核验的 Cargo 依赖边：
   - `bamboo-tools` → `bamboo-agent-core`, `bamboo-domain`, `bamboo-infrastructure`
   - `bamboo-engine` → `bamboo-domain`, `bamboo-infrastructure`, `bamboo-agent-core`, `bamboo-tools`
   - `bamboo-server` → 以上全部
   - root crate → 以上全部 + `bamboo-server`
2. **防分叉：**SDK runner 绝不能复制 `run_spawn_job` 那 330 行 spawn/finalize 逻辑，
   必须复用唯一的 canonical spawn 路径。server 必须*改接到*该 runner 上，最终只留一份
   实现。
3. **所有既有测试保持绿。**事件、提示词、tool-policy、模型解析不得有任何行为漂移。

### explorer 之间的矛盾——已解决

| # | 矛盾 | 解决（重新读码后） |
|---|----------|-----------------------------------|
| C1 | Explorer 1 提议 `sdk/runner.rs`，带一个肥大的 `RuntimeDeps`/`RunOutcomeStream`（broadcast）。Explorer 5 提议 `sdk/spawn.rs`，把 `run_spawn_job` 抽成 `spawn_profile_child`。 | **合并。**单一模块 `crates/bamboo-engine/src/sdk/`。*核心*抽取是 `spawn.rs`（重构 `run_spawn_job`，使其主体变成可复用的 `run_child_spawn(ctx, job)`，接收现有的 `SpawnContext` + `SpawnJob`）。`runner.rs` 是覆盖在该核心之上的薄易用门面（`ProfileRunner`）。**不新增 `RuntimeDeps` 上帝结构体**——复用 `SpawnContext`，它已经恰好持有全部依赖（agent、tools、caches、router、completion_handler）。Explorer 1 的 14 字段 `RuntimeDeps` 被否决，理由是它是 `SpawnContext` 的冗余平行物。 |
| C2 | Explorer 1：`ExecuteRequest` 有 23 个字段，含 `fast_model*`、`background_model*`、`summarization_model*`。Explorer 4："21 optional"。 | **以代码为准。**真实的 `ExecuteRequest`（spawn.rs:563-587）是拆分的 provider 字段：`fast_model` + `fast_model_provider`、`background_model` + `background_model_provider`、`summarization_model` + `summarization_model_provider`。任何 builder 都必须枚举*实际*字段。runner 把它们全部置为 `None`（与当前 spawn 行为一致）。 |
| C3 | Explorer 3：把 `loader.rs` 和 `builtin.rs` 都移到 engine。Explorer 6 风险提示：loader 可以留在 server。 | **两个都移到 `crates/bamboo-engine/src/profiles/`。**`loader.rs` 只用 `std::fs`、`serde`、`thiserror`、`bamboo_domain`——engine 里全都有。留在 server 会把 profile 体系拆散到多个 crate。server 保留一层薄的 re-export shim 以向后兼容。 |
| C4 | Explorer 2：把 `PolicyAwareToolExecutor` 移到 `bamboo-tools`。Explorer 6 风险：需要可注入的 session 缓存。 | **移入 `bamboo-tools`。**已核验：它只依赖 `bamboo_agent_core::tools`、`bamboo_agent_core::Session`、`bamboo_domain::subagent`、`tokio::sync::RwLock`——`bamboo-tools` 里全都有（它本来就依赖 `bamboo-domain`）。`Arc<RwLock<HashMap<String, Session>>>` 是构造函数参数，本就可注入。无风险。 |
| C5 | Explorer 3 的测试引用修复：把 `crate::session_app::...` 改成 `bamboo_engine::session_app::...`。 | **移动之后就是错的。**一旦 `builtin.rs` 位于 `bamboo-engine` *内部*，路径保持 `crate::session_app::child_session::CHILD_SYSTEM_PROMPT`（crate 相对路径；engine 就是所有者）。不要改成 `bamboo_engine::`（自引用）。已核验常量位于 `crates/bamboo-engine/src/session_app/child_session/{helpers.rs,mod.rs}`。 |
| C6 | Explorer 2：抽取纯函数 `infer_provider(model_name) -> Option<String>` 和统一的 `resolve_model`。Explorer 6 提示模型优先级是 `Config.subagent_models[id] > model_hint.model_ref > model_hint.tier`。 | **在 `bamboo-engine/src/model_config_helper.rs` 新增纯 helper `infer_provider` + `resolve_model`**（engine 已经拥有 `resolve_subagent_model_ref`）。不要移进 `bamboo-tools`（那需要 `Config`/`ProviderRegistry`，会把 infrastructure 的模型路由不必要地拖进 tools）。现有 `resolve_subagent_model*` 原样保留；新 helper 是纯增量。 |
| C7 | Explorer 4/6：在根上加 `Agent::researcher()`/`.coder()`，需要 profile 注册表。 | **根门面 re-export `bamboo_engine::profiles`**（迁移之后）。`.researcher()`/`.coder()`/`.from_profile()` 从 `builtin_profiles()` 解析，并设置 builder 的 system-prompt/tool-policy。根目录不重复定义 profile。 |

### 非目标（明确推迟）

- 移除双重 tool-policy 执行（schema `disabled_tools` + `PolicyAwareToolExecutor` 运行时兜底）。两者都保留；文档写明权威归属。（技术债 TD-7。）
- 把 `ChildStatus` 字符串字面量在 wire 层改成枚举。内部用枚举可以；wire 字符串保持不变。（技术债 TD-5。）
- A2A/外部 runner 变更。

---

## 1. 目标架构（终态）

```
bamboo-domain
  subagent/{model.rs, registry.rs}        # SubagentProfile、ToolPolicy、disabled_tools_for_profile（不变）

bamboo-tools
  policy_aware.rs                          # PolicyAwareToolExecutor（从 server 移入此处）

bamboo-engine
  model_config_helper.rs                   # + infer_provider()、+ resolve_model()（纯增量）
  profiles/{mod.rs, builtin.rs, loader.rs} # 从 server/subagent_profiles 移入此处
  sdk/{mod.rs, runner.rs, spawn.rs}        # 新增：ProfileRunner + run_child_spawn 核心
  runtime/execution/spawn.rs               # run_spawn_job 变为 sdk::spawn::run_child_spawn 的薄调用方

bamboo-server
  tools/policy_aware.rs                     # 删除 → re-export shim（pub use bamboo_tools::PolicyAwareToolExecutor）
  subagent_profiles/mod.rs                  # → re-export shim（pub use bamboo_engine::profiles::*）
  tools/child_session_adapter.rs           # enqueue_child_run 行为不变；仍构建 SpawnJob
                                            # （scheduler 仍驱动 run_spawn_job → 现在走 sdk 核心）

root crate (bamboo_agent)
  src/agent/{mod.rs, builder.rs, tools.rs, execute_request.rs}  # 易用门面
  src/lib.rs                                # 清理后的 re-export + pub use agent::*
```

**防分叉保证：**`run_spawn_job` 与 `ProfileRunner::run` 都汇入
`sdk::spawn::run_child_spawn(ctx: SpawnContext, job: SpawnJob)`。spawn/execute/finalize
实现全局仅此一份。

---

## 2. 按依赖排序的阶段

每个阶段以一个 **GATE**（`cargo build` + `cargo test`）收尾。阶段之间在 gate 边界严格
串行。阶段内部，标注 **[PAR]** 的步骤改动互不相交的文件，可并行推进；**[SEQ]** 步骤
存在编译依赖，必须串行。

> 所有 cargo 命令都从仓库根目录 `/Users/bigduu/Workspace/TauriProjects/zenith/bamboo` 运行。
> 使用 `cargo build --workspace` 与 `cargo test --workspace`（或按 crate 用 `-p` 加快内层循环）。

---

### 阶段 0 — 预检技术债（无行为变化，风险最低）

纯注释/文档修正；先做最安全，且可并行。

- **[PAR] S0.1** 修正过期的 `bamboo-application-agent` 引用：
  - `src/agent/mod.rs:4` → "via bamboo-engine"
  - `crates/bamboo-domain/src/session/hook_types.rs:5`
  - `crates/bamboo-domain/src/session/composition/condition.rs:4`
  - `crates/bamboo-domain/src/session/composition/mod.rs:4`
- **[PAR] S0.2** `src/lib.rs:48` 删除 "Placeholder modules (will be populated during migration)" 注释（该模块将随本 PR 填充）。

**GATE 0：**`cargo build --workspace`（只改注释；仍须可编译）。

---

### 阶段 1 — 桥接（尚不引入 engine/runner 依赖）

两个相互独立的桥接迁移。S1.A 与 S1.B 之间 **[PAR]**（文件不相交、crate 不相交）。

#### S1.A — 把 `PolicyAwareToolExecutor` 移入 `bamboo-tools`（解决 C4）

- **[SEQ] S1.A.1** 新建 `crates/bamboo-tools/src/policy_aware.rs`：把
  `crates/bamboo-server/src/tools/policy_aware.rs` 的完整模块体（含测试）移过来。
  这些 import 在 tools 中本就合法（`bamboo_agent_core::tools::*`、`bamboo_agent_core::Session`、
  `bamboo_domain::subagent::{SubagentProfileRegistry, ToolPolicy}`、`tokio::sync::RwLock`）。
- **[SEQ] S1.A.2** `crates/bamboo-tools/src/lib.rs`：新增 `pub mod policy_aware;` 与
  `pub use policy_aware::PolicyAwareToolExecutor;`。
- **[SEQ] S1.A.3** 把 `crates/bamboo-server/src/tools/policy_aware.rs` 的内容替换为
  re-export shim：`pub use bamboo_tools::PolicyAwareToolExecutor;`（保住
  `crate::tools::PolicyAwareToolExecutor` 路径在 `crates/bamboo-server/src/tools/mod.rs:29`
  与 builder.rs:240 处继续可用）。*备选：*删除文件 + 把 `tools/mod.rs` 的 re-export
  改指向 `bamboo_tools`。shim 风险更低。

#### S1.B — 在 engine 中新增纯模型 helper（解决 C6）

- **[SEQ] S1.B.1** `crates/bamboo-engine/src/model_config_helper.rs`：新增
  `pub fn infer_provider(model_name: &str) -> Option<String>`（模式：`claude*`→`anthropic`、
  `gpt*`/`o[0-9]*`→`openai`、`gemini*`→`gemini`，否则 `None`）。把目前散落在各 resolve
  函数里的隐式模式匹配抽出来，改调这个 helper（重构，行为不变）。
- **[SEQ] S1.B.2** 新增 `pub fn resolve_model(model_hint: &ModelHint, provider_name: &str,
  config: &Config, provider_registry: &Arc<ProviderRegistry>) -> Option<ResolvedModel>`，
  遵循优先级 **`model_hint.model_ref` > `model_hint.tier` > 回退链**
  （`subagent_models[type]` → `sub_agent` → `fast` → `chat`）。复用现有
  `resolve_subagent_model_ref`。现有 `resolve_subagent_model*` 原样不动。

**GATE 1：**`cargo build --workspace`，然后
`cargo test -p bamboo-tools -p bamboo-engine -p bamboo-server`。
既有 9 个 `policy_aware` 测试必须*在新家*通过；engine 模型测试不变。

---

### 阶段 2 — engine SDK runner（依赖阶段 1 的 helper）

把 canonical spawn 路径重构成可复用核心，再加上易用的 runner 门面。**基本全为 [SEQ]**
（都涉及 `bamboo-engine/src/sdk` + `spawn.rs`）。

- **[SEQ] S2.1** 新建 `crates/bamboo-engine/src/sdk/mod.rs`（`pub mod runner; pub mod spawn;`）。
- **[SEQ] S2.2** 新建 `crates/bamboo-engine/src/sdk/spawn.rs`。把 `run_spawn_job` 的**主体**
  （当前在 `runtime/execution/spawn.rs:320-651`）移入
  `pub async fn run_child_spawn(ctx: SpawnContext, job: SpawnJob) -> Result<(), String>`。
  必须逐项原样保留：
  - SubAgentStarted 由 *adapter* 发出（不在这里发）——不变。
  - 事件转发器 + 5s 心跳任务、watchdog、runner 预留。
  - `ExecuteRequest` 构造包含全部真实字段，含拆分的 provider 字段
    （`fast_model_provider`、`background_model_provider`、`summarization_model_provider`）
    ——见 C2。`disabled_tools = job.disabled_tools.map(|v| v.into_iter().collect())`。
  - 带状态字符串 `completed|cancelled|error|skipped|timeout` 的
    `publish_child_completion_parts` 终止路径。
- **[SEQ] S2.3** `runtime/execution/spawn.rs`：`run_spawn_job` 变成一行委托：
  `crate::sdk::spawn::run_child_spawn(ctx, job).await`。（`SpawnScheduler` 队列机制原样保留。）
  **防分叉检查点。**
- **[SEQ] S2.4** 新建 `crates/bamboo-engine/src/sdk/runner.rs`：
  - `pub struct ProfileRunner { ctx: SpawnContext }`——**复用 `SpawnContext`**，不新建
    `RuntimeDeps`（解决 C1）。
  - `pub fn profile_runner(ctx: SpawnContext) -> ProfileRunner`。
  - `pub struct RunProfileInput { child_session_id, parent_session_id, model, /* derived */ }`
    ——保持最小；任务提示词 + system prompt 已经存在于持久化的子 session 中（与真实
    spawn 语义一致：`initial_message` 为空，由子 session 中最后一条 user 消息驱动执行）。
  - `impl ProfileRunner { pub async fn run_profile(&self, profile: &SubagentProfile, input: RunProfileInput) -> Result<(), String> }`：
    通过 `bamboo_domain::subagent::disabled_tools_for_profile(&profile.tools, &tool_names)`
    计算 `disabled_tools`，构建 `SpawnJob`，调用 `run_child_spawn(self.ctx.clone(), job)`。
  - 流式变体 `run_profile_stream` 返回 `broadcast::Receiver<AgentEvent>`，取自
    `ctx.session_event_senders` 中该子 id（复用现有 broadcast 基础设施；不要发明
    `RunOutcomeStream`/`status_rx` mpsc——解决 C1）。
- **[SEQ] S2.5** `crates/bamboo-engine/src/lib.rs`：新增 `pub mod sdk;` 与
  `pub use sdk::runner::{ProfileRunner, profile_runner, RunProfileInput};`、
  `pub use sdk::spawn::run_child_spawn;`。

**GATE 2：**`cargo build --workspace`，然后 `cargo test -p bamboo-engine -p bamboo-server`。
29 个 `sub_agent.rs` 测试（尤其是约 797 行的
`create_emits_sub_agent_started_event_after_queueing`）必须保持绿——它们走的正是
scheduler→`run_spawn_job`→`run_child_spawn` 这条未变路径。新增 engine 测试 S-T2.*（见 §4）。

---

### 阶段 3 — profile 迁移（依赖 engine 的 session_app；与 sdk 相互独立）

> 日历时间上可与阶段 2 重叠（文件不同），但 gate 排在阶段 2 *之后*，以保持一个干净的
> engine 构建 gate。内部步骤均为 **[SEQ]**。

- **[SEQ] S3.1** 新建 `crates/bamboo-engine/src/profiles/builtin.rs`：从
  `crates/bamboo-server/src/subagent_profiles/builtin.rs` 逐字复制。**保持**测试引用
  `crate::session_app::child_session::{CHILD_SYSTEM_PROMPT, PLAN_AGENT_SYSTEM_PROMPT}`
  不变——engine 拥有 `session_app`，因此能正确解析（解决 C5）。把过期的模块文档注释
  "(future) FilteredExecutor" 更新为指向 `bamboo_tools::PolicyAwareToolExecutor`。
- **[SEQ] S3.2** 新建 `crates/bamboo-engine/src/profiles/loader.rs`：从
  `crates/bamboo-server/src/subagent_profiles/loader.rs` 移入。import 不变
  （`bamboo_domain::subagent::*`、`std::fs`、`thiserror`）。更新
  "consumer typically bamboo-server" 的文档注释。
- **[SEQ] S3.3** 新建 `crates/bamboo-engine/src/profiles/mod.rs`：
  `pub mod builtin; pub mod loader; pub use builtin::builtin_profiles; pub use loader::{load_registry, LoaderError};`。
- **[SEQ] S3.4** `crates/bamboo-engine/src/lib.rs`：新增 `pub mod profiles;` +
  `pub use profiles::{builtin_profiles, load_registry, LoaderError};`。
- **[SEQ] S3.5** 把 `crates/bamboo-server/src/subagent_profiles/mod.rs` 改为 shim：
  `pub use bamboo_engine::profiles::{builtin_profiles, load_registry, LoaderError};
  pub mod builtin { pub use bamboo_engine::profiles::builtin::*; }`（保住
  sub_agent.rs:710 处的 `crate::subagent_profiles::builtin::builtin_profiles` 与
  builder.rs:228 处的 `crate::subagent_profiles::load_registry`）。从 server 删除如今
  重复的 `builtin.rs`/`loader.rs`。

**GATE 3：**`cargo build --workspace`，然后
`cargo test -p bamboo-engine -p bamboo-server`。
迁移后的 profile 测试（6 个 builtin + 7 个 loader）在 engine 中运行并通过；server
路由测试 `GET /v1/subagent_profiles`（routes/tests.rs）保持绿。

---

### 阶段 4 — 根门面（`src/agent/`）（依赖 engine 的 profiles + sdk + tools）

所有新增/修改文件都在 `src/agent/` 下。三个新文件之间 **[PAR]**（S4.1 tools.rs、
S4.2 execute_request.rs、S4.3 builder.rs 互不相交），随后 **[SEQ]**：S4.4 mod.rs 负责
接线，S4.5 lib.rs。

- **[PAR] S4.1** `src/agent/tools.rs`：`pub struct ToolSpec { name, description, disabled }`
  + 常量映射到 `bamboo_domain::tool_names::BUILTIN_TOOL_NAMES` 中的**真实**名称
  （对照该常量数组核验；不要手写清单）。re-export canonical 清单。
- **[PAR] S4.2** `src/agent/execute_request.rs`：`ExecuteRequestBuilder` 转发到
  `bamboo_engine::ExecuteRequest`，覆盖全部真实字段（3 个必填 + 其余，含按 C2 拆分的
  provider 字段）。默认值与当前 spawn 默认一致（`None`）。
- **[PAR] S4.3** `src/agent/builder.rs`：包装 `bamboo_engine::AgentBuilder`。易用方法：
  `.from_profile(&SubagentProfile)`（设置 system_prompt + tool policy）、
  `.researcher()`/`.coder()` 等（经 `bamboo_engine::profiles::builtin_profiles()` 解析，
  解决 C7）、`.model()`、`.instruction()`、`.tools()`、`.api_key()`、
  `.with_defaults_for_data_dir(PathBuf)`（组装那 8 个依赖：`Config::from_data_dir`/
  `Config::new`、`JsonlStorage::new`+`init`、`SkillManager::new`+`initialize`、带
  `SqliteMetricsStorage` 的 `MetricsCollector::spawn`（已核验存在：
  `bamboo_engine::SqliteMetricsStorage`）、`create_provider`、
  `BuiltinToolExecutor::new_with_config`）。
- **[SEQ] S4.4** `src/agent/mod.rs`：替换纯透传。定义
  `pub struct Agent { inner: Arc<AgentRuntime> }`，带 `from_runtime`/`builder`/`run`/
  `run_stream`/`storage`/`persistence`；`mod builder; mod tools; mod execute_request;`、
  `pub use {builder::AgentBuilder, tools::*, execute_request::ExecuteRequestBuilder};`、
  为使用者提供 `pub use bamboo_engine::profiles;`。保留现有便利类型 re-export
  （Session、Message 等）。
- **[SEQ] S4.5** `src/lib.rs:63`：`pub use agent::{Agent, AgentBuilder};`（现在来自新
  包装器）。需要的话新增 `pub use agent::profiles;`。

**GATE 4：**`cargo build --workspace`，然后 `cargo test --workspace`。
新增根 SDK 测试 S-T4.*（见 §4）。

---

### 阶段 5 — server 改接到 runner（防分叉落地）

server 的路径已经流经 `run_spawn_job` →（如今）`run_child_spawn`，核心在阶段 2 之后
就已统一。本阶段**可选地**让 `ChildSessionAdapter` 直接调用 `ProfileRunner` 而不是
`scheduler.enqueue`，*前提是完全保住异步 enqueue 语义与 SubAgentStarted 顺序*。
**保守默认：调度器路径维持原状**（它已调用统一核心），只需核验不存在第二份实现。

- **[SEQ] S5.1** 审计：`grep` 检查 `sdk::spawn::run_child_spawn` 之外是否还残留内联
  spawn/execute/finalize 逻辑。必须一处不剩。
- **[SEQ] S5.2**（可选）重构 `enqueue_child_run`，经由已有的
  `disabled_tools_for_profile` 调用（318-339 行）构造 `SpawnJob`——无需改动；文档写明
  adapter 仍是 SpawnJob 工厂 + 父等待登记器。
- **[SEQ] S5.3** 确认 `PolicyAwareToolExecutor` 仍在 `builder.rs:240` 经新的
  `bamboo_tools` 路径包装子工具（shim 让这一步保持透明）。

**GATE 5：**`cargo test --workspace`。**关键不变量检查：**
- SubAgentStarted 在父等待持久化*之后*发出（sub_agent.rs:797）。
- Allowlist profile 子代：工具不在 schema 中（disabled_tools），且执行时被拦截
  （policy_aware）。新增集成测试 S-T5.2。

---

### 阶段 6 — 文档

- **[PAR] S6.1** 本文档（已在 `docs/design/ergonomic-sdk-plan.md`）。
- **[PAR] S6.2** 给 `src/agent/mod.rs` 与 `bamboo-engine/src/sdk/mod.rs` 增加模块级文档，
  描述公共 SDK 面与防分叉不变量。
- **[PAR] S6.3** 更新 `bamboo-engine/src/profiles/{mod,loader}.rs` 与
  `bamboo-tools/src/policy_aware.rs` 的文档注释，反映新归属。

**GATE 6：**`cargo doc --workspace --no-deps`（无失效的 intra-doc 链接）。

---

## 3. 并行化小结

| 可并行 | 必须串行 |
|---------------------|--------------------|
| S1.A 与 S1.B（不同 crate） | 阶段 2 *内部*的一切（共享 sdk/spawn 文件） |
| S0.1 与 S0.2 | S2.2 → S2.3（先抽取，再委托） |
| S4.1 与 S4.2 与 S4.3（互不相交的新文件） | 每个 GATE 处 S2.x → S3.x → S4.x → S5.x |
| S6.1 与 S6.2 与 S6.3 | S4.4 在 S4.1/4.2/4.3 之后（mod 负责接线） |

阶段之间由 GATE 严格排序。两名开发者可以同时认领 S1.A 与 S1.B；engine runner 作者
（阶段 2）会阻塞门面作者（阶段 4）。

---

## 4. 测试计划

### 必须保持绿（回归）

- `bamboo-domain`：全部 `subagent/model.rs` policy 测试 + `registry.rs`（8 个）——不动。
- `bamboo-tools`（迁移后）：9 个 `PolicyAwareToolExecutor` 测试
  （`inherit_policy_forwards_all_calls`、`allowlist_permits/blocks`、
  `denylist_blocks/permits`、`missing_session_id_falls_through`、
  `unknown_session_falls_through`、`missing_subagent_type_metadata_falls_through`、
  `execute_without_context_forwards`）。
- `bamboo-engine`：`model_areas.rs`（9 个）、`model_config_helper.rs`、child_session
  `tests.rs`（10 个）、runtime `tests.rs`（5 个）。
- `bamboo-engine`（迁移后）：6 个 builtin-profile + 7 个 loader 测试（含针对
  `CHILD_SYSTEM_PROMPT`/`PLAN_AGENT_SYSTEM_PROMPT` 的提示词漂移交叉校验）。
- `bamboo-server`：29 个 `sub_agent.rs` 测试、`routes/tests.rs` profile 列表冒烟、
  `policy_aware` shim re-export 可编译。

### 新增测试

- **S-T1.1** `infer_provider`：claude/gpt/o-series/gemini/unknown 映射。
- **S-T1.2** `resolve_model`：优先级 `model_ref` > `tier` > 回退链。
- **S-T2.1** `run_child_spawn` 集成：父 + 子 session，断言 SubAgentStarted（adapter）→
  SubAgentEvent → SubAgentCompleted 的顺序；子状态已持久化（completed）。
- **S-T2.2** 用 Allowlist profile 跑 `ProfileRunner::run_profile`：断言 `disabled_tools`
  排除了非白名单名称（schema 层）。
- **S-T2.3** 用 `ToolPolicy::Inherit` 跑 `run_profile`：`disabled_tools` 为空。
- **S-T2.4** runner 处的模型优先级：`model_override` 优先于 session 模型。
- **S-T2.5** Watchdog 超时 → SubAgentCompleted status=`timeout`（复用现有 watchdog 管线）。
- **S-T4.1** `Agent::builder().researcher().model("m").build()` → 解析出的
  system_prompt 与 researcher profile 匹配 + model override 生效。
- **S-T4.2** `ExecuteRequestBuilder` 往返：全部必填字段强制填写，全部可选默认 `None`。
- **S-T4.3** `with_defaults_for_data_dir(tmp)`：用 NoopProvider/mock 构建出 `Agent`；
  `SkillManager.initialize` + `MetricsCollector.spawn` 成功。
- **S-T5.2** 端到端 policy：子代 `subagent_type=researcher`（只读白名单）→ Edit/Write
  执行时被拦截，*且*不出现在 schema 中。

> SDK 集成测试使用 Noop/mock 的 `LLMProvider`（模型解析测试中的既有模式），避免
> 网络 I/O。

---

## 5. 技术债清理（伴随重构，在相关阶段内完成）

- **TD-1（阶段 0）：**移除 4 处过期的 `bamboo-application-agent` 注释 +
  `src/lib.rs:48` 的占位注释。
- **TD-2（阶段 0/4）：**把重复的 Agent re-export 链（`src/lib.rs:63` →
  `agent/mod.rs:24` → `bamboo_engine`）收敛进新包装器。
- **TD-3（阶段 1）：**把 `model_config_helper.rs` 中散落的 provider 模式匹配抽进唯一的
  `infer_provider`。
- **TD-4（阶段 3）：**把 builtin.rs 的 "(future) FilteredExecutor" 注释更新为
  `PolicyAwareToolExecutor`；更新 loader.rs 的 "consumer typically bamboo-server"。
- **TD-5（推迟、已记录）：**内部 `ChildStatus` 枚举 vs wire 字符串——wire 上保持字符串；
  留档备查。
- **TD-6（阶段 4）：**新增 `ExecuteRequestBuilder`，让使用者不必直面原始的多字段
  `ExecuteRequest`。
- **TD-7（阶段 5、已记录）：**双重 tool-policy 执行（schema `disabled_tools` 是
  *发现阶段的权威*；`PolicyAwareToolExecutor` 是*执行时的安全网*）。写进文档；不要移除。
- **TD-8（阶段 2）：**`disabled_tools_for_profile` 需要调用方传入 `all_tool_names`；
  文档写明 `SpawnContext.tools.list_tools()` 是 canonical 来源，让调用方不再单独穿引
  `tool_names: Vec<String>`。

---

## 6. 反向依赖风险登记表

| 风险 | 缓解 |
|------|-----------|
| 把 `PolicyAwareToolExecutor` 移到 `bamboo-tools`，一旦它引用任何 `bamboo-server`/`bamboo-engine` 符号就会失败。 | 已核验：只有 `agent-core` + `domain` + tokio。安全，无反向边。 |
| 把 `profiles` 移入 engine：server 仍需要它们 → server 本就依赖 engine。无新边；*移除*的是 server 自有的逻辑。 | re-export shim 保住 server 路径；无循环 import（类型归 domain 所有）。 |
| 根门面 `.with_defaults_for_data_dir` 把 `bamboo-server` 拖进 agent builder。 | 只从 `infrastructure`/`engine`/`tools` 构建依赖。`bamboo-server` 不得进入 `Agent` builder 路径。 |
| `sdk::runner` 复用 `SpawnContext` 可能诱使引入 server 的 `AppState`。 | `SpawnContext` 位于 engine，与 server 无关（completion_handler 是 trait object）。无 `AppState` 引用。 |
| engine 中的 `infer_provider`/`resolve_model` 需要 `ProviderRegistry`（infrastructure）——没问题（engine→infra 已存在），但绝不能落进 `bamboo-tools`。 | 按 C6，helper 留在 engine。 |

---

## 7. 防分叉核验清单（在 GATE 5 执行）

1. `grep -rn "ExecuteRequest {" crates/bamboo-engine/src` → 只应命中 `sdk/spawn.rs`
   （以及既有的根 session execute 路径；sub-agent 路径唯一）。
2. `run_spawn_job` 主体是对 `run_child_spawn` 的单行委托。
3. `ProfileRunner::run_profile` 构造 `SpawnJob` + 调用 `run_child_spawn`；没有内联
   execute/finalize。
4. `bamboo-engine` 之外没有重复的 `builtin_profiles()` / `load_registry()`
   （server shim 只做 re-export）。
5. `bamboo-tools` 之外没有重复的 `PolicyAwareToolExecutor` 实现。
