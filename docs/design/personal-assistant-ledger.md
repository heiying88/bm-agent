# 个人助理能力：前瞻记忆与记录台账

**状态：**已完整实现（阶段 1–7）
**日期：**2026-07-13
**范围：**Bamboo 如何以**一个通用能力**成为个人助理代理（待办、任务分解、日程安排、提醒），而不是一堆功能专属的工具。

---

## 0. 本文回答的问题

> "Bamboo 需要个人助理能力——待办清单、任务分解、日程安排。抽象来看，这是否只是长期 + 短期记忆的一种应用？是否应该为这些事件划出一块专用磁盘区域？请设计一个*通用*能力，而不仅仅是那几个具名特性。"

**简答：**它*几乎*是记忆系统的一种应用，但不完全是。Bamboo 既有的记忆是**回溯性的**——记录已经发生的事（事实、偏好、决策），以自由文本 markdown 保存，由 auto-dream 整合、由 gardener（园丁）维护。个人助理的工作是**前瞻记忆**：关于*未来*的结构化记录，具有**生命周期**（open → done/cancelled/expired）、**时间语义**（到期、定时、提醒时刻、重复）与**关系**（分解、依赖）。自由文本的持久记忆无法回答"明天有什么到期？"，也无法驱动一次提醒；todo 不是事实，而是一个带状态的承诺。

因此这个通用能力是第三个存储——**记录台账（Record Ledger）**——它与持久记忆并列，复用其存储惯例与磁盘约定，并接入 Bamboo 已有的四台机器：提示注入、调度引擎、通知管道与后台整合循环。待办、事件、提醒、习惯与任务分解都成为同一个模型上的*记录种类*。

而且没错：它拥有自己的版本化磁盘区域 `~/.bamboo/ledger/v1/`，与 `~/.bamboo/memory/v1/` 平行。

---

## 1. 现状盘点

这些构件已经异常扎实。下面列出的任何东西都不需要重建——本设计主要是在*连线*。

| 子系统 | 作用 | 关键位置 |
|---|---|---|
| **会话任务列表**（`Task` 工具） | 丰富的 todo 模型：status、`depends_on`、`parent_id`、阶段、优先级、完成标准、证据、阻塞项、状态迁移历史。渲染进提示。 | `crates/core/bamboo-domain/src/session/task.rs`；持久化经 `crates/engine/bamboo-engine/src/runtime/runner/tool_execution/task/taskwrite.rs` |
| **规划** | 新工作委派给只读的规划器 child；已持久化的旧版 PlanMode 会话保留退出握手与 `~/.bamboo/plan/<slug>/` 下的 PlanStore 产物。 | `crates/app/bamboo-server-tools/src/plan.rs`; `crates/engine/bamboo-tools/src/tools/exit_plan_mode.rs`; `crates/infra/bamboo-memory/src/plan_store.rs` |
| **记忆系统** | 会话笔记（短期，`memory/v1/sessions/<id>/note/*.md`）+ 持久记忆（长期，`memory/v1/scopes/{global,projects/<key>}/topics/*.md`，带 YAML frontmatter 的 markdown，BM25 召回，审计日志）+ Dream 笔记本（派生视图）。 | `crates/infra/bamboo-memory/src/memory_store/` |
| **后台循环** | auto-dream（每 30 分钟从近期会话提取持久记忆）与 gardener（大块拆分 / 去重 / 容量整理趟，确定性预筛 → 只在有活时才用 LLM）。 | `crates/engine/bamboo-engine/src/{auto_dream,gardener}.rs`，在 `crates/app/bamboo-server/src/app_state/builder.rs:442,455` 处 spawn |
| **提示注入** | `## External Memory (Persistent)` 易变块，按优先级分层：观察到的状态 > 会话笔记 > 最相关的 3 条持久记忆 > 项目索引 > dream 摘要。对缓存友好（不进入被缓存的 system 前缀）。 | `crates/engine/bamboo-engine/src/runtime/runner/prompt_context/external_memory.rs` |
| **调度引擎** | Interval/Daily/Weekly/Monthly/Cron 触发器、错失（misfire）与重叠策略、逐次运行记录；**触发会创建一个真实的代理会话**，并在 `auto_execute` 下以无头方式运行该循环、接上通知中继。持久化到 `~/.bamboo/schedules.json`。面向 LLM 的 `scheduler` overlay 工具。 | `crates/app/bamboo-server/src/schedule_app/`；领域模型在 `crates/core/bamboo-domain/src/schedule/domain.rs` |
| **通知** | 策略引擎（分类、去重）+ 下发通道：桌面弹窗、ntfy、bark（手机推送）。可用于无头定时运行。 | `crates/infra/bamboo-notification/`; `crates/app/bamboo-server/src/notify_sinks/` |
| **Overlay 工具** | 服务端有状态工具（`memory`、`scheduler`、`notify`、`load_skill`、`SubAgent`、……）在内置执行器之上链式组合——这是需要存储/服务的工具的接缝。 | `crates/app/bamboo-server/src/app_state/tools.rs` |
| **Skills** | 提示片段 + 工具引用的捆绑包，带渐进式披露（`load_skill`）。内置项由 `/builtin_skills` 预置。 | `crates/infra/bamboo-skills/` |

## 2. 缺口分析——为什么这些部件还拼不出一个助理

1. **任务随会话树一起消亡。**`TaskList` 以*根会话 id* 为键，存储在 `Session` 记录上。今天让 Bamboo"提醒我续签护照"，这个 todo 只活在一次对话里；明天的会话一无所知。没有*用户级*的任务存储。
2. **持久记忆的形状不适合承诺。**它是自由文本的原子*事实*，只有新鲜/过时语义。没有状态机、没有到期日、没有时间范围查询、也无法从中触发提醒。把 todo 塞进去会污染召回，而且照样产生不了提醒。
3. **调度引擎与任务无关。**它能"每天早上 8 点"运行，但调度不与任何记录关联——完成一个 todo 无法取消它的提醒，已触发的提醒会话也没有一个结构化句柄指回它所针对的事物。它还缺少**一次性**触发器（"在 2026-07-20T09:00"），而这恰恰是最常见的提醒形态。
4. **没有东西在提取承诺。**auto-dream 从对话中提取*事实*；没有人把"我答应周五发报告"提取成任何可执行的东西。
5. **提示里没有日程。**助理无法主动说"你今天有两件事到期"，因为没有东西把与时间相关的未完成项注入上下文。

## 3. 核心抽象

三个记忆视界，其中一个是新的：

| 视界 | 内容 | 存储 | 已存在？ |
|---|---|---|---|
| **工作 / 短期** | 当前对话的连续性：会话笔记、会话 `TaskList`、plan 产物 | Session 存储 + `memory/v1/sessions/` + `plan/` | ✅ |
| **前瞻（新增）** | 面向未来的结构化记录：todo、事件、提醒、习惯——生命周期 + 时间 + 关系 | **`ledger/v1/`** | ❌ 本设计 |
| **回溯 / 长期** | 事实、偏好、决策；dream 摘要 | `memory/v1/scopes/` | ✅ |

各视界之间的流动才是有趣的部分：

```
conversation ──(Task tool / explicit ask / extractor)──▶ LEDGER record
session TaskList ──("promote" action)────────────────▶ LEDGER record (survives session)
LEDGER record.remind_at ──(schedule bridge)──────────▶ ScheduleSpec ──fires──▶ headless session + push notification
LEDGER agenda view ──(prompt injection)──────────────▶ every session's context ("due today: …")
LEDGER completed/expired ──(ledger gardener)─────────▶ distilled into durable memory ("user renews passport every 10y", habit stats)
```

通用的含义是：除了一个 `kind` 标签和使用了哪些时间字段，台账并不知道"todo"是什么。新的助理行为（习惯打卡、生日、服药、邮件跟进）都是新的*种类* + 视图，而不是新的子系统。

## 4. 领域模型（`bamboo-domain`）

新模块 `crates/core/bamboo-domain/src/ledger/`。刻意复用既有词汇（`TaskPriority`、`MemoryScope`、`CreatedBy`、`ScheduleTrigger`），而不是发明平行的枚举。

```rust
pub enum RecordKind { Todo, Event, Reminder, Habit, Custom(String) }

pub enum RecordStatus { Open, InProgress, Blocked, Done, Cancelled, Expired }

/// 时间语义。全部可选——一条随手写给自己的备忘可以全都没有。
pub struct RecordTime {
    pub due_at: Option<DateTime<Utc>>,        // 截止时间（todo）
    pub starts_at: Option<DateTime<Utc>>,     // 日历事件
    pub ends_at: Option<DateTime<Utc>>,
    pub remind_at: Vec<DateTime<Utc>>,        // 显式提醒时点
    pub recurrence: Option<ScheduleTrigger>,  // 复用调度触发器模型
    pub timezone: Option<String>,
}

pub struct RecordRelations {
    pub parent_id: Option<String>,            // 分解树
    pub depends_on: Vec<String>,              // 先后顺序
    pub related: Vec<String>,                 // 记忆 id、会话 id、url
}

pub struct RecordSource {
    pub session_id: Option<String>,           // 出处：来自哪次对话
    pub created_by: CreatedBy,                // 用户 | 代理 | 后台循环
    pub excerpt: Option<String>,              // 促成它的那句话
}

pub struct LedgerRecord {
    pub id: String,
    pub kind: RecordKind,
    pub title: String,
    pub body: String,                          // markdown
    pub status: RecordStatus,
    pub priority: TaskPriority,                // 复用
    pub scope: MemoryScope,                    // 复用：Global（个人）| Project
    pub time: RecordTime,
    pub relations: RecordRelations,
    pub source: RecordSource,
    pub tags: Vec<String>,
    pub schedule_ids: Vec<String>,             // 托管的 ScheduleSpec（见 §7）
    pub transitions: Vec<RecordTransition>,    // 状态历史，TaskTransition 风格
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

**任务分解不是一个特性——它就是 `parent_id` + `depends_on`**，正是 `TaskItem` 已在会话内验证过的形状。"把这个拆成几步" = 代理写入子记录。一棵单根的记录树*本身*就是一份能跨会话存续的项目计划。

## 5. 存储：`~/.bamboo/ledger/v1/`（专用磁盘区域）

新模块 `crates/infra/bamboo-memory/src/ledger_store/`（与 `memory_store` 同一个 crate——共用 `atomic_fs`、路径解析惯例、审计日志风格与 frontmatter 解析器；单独建 crate 只会白添一条依赖边）。

```
~/.bamboo/ledger/v1/
└── scopes/
    ├── global/                          # personal life — the default scope
    │   ├── records/<record_id>.md       # YAML frontmatter + markdown body
    │   ├── indexes/
    │   │   ├── by_time.json             # sorted (due|starts|remind) → id; agenda queries
    │   │   ├── by_status.json           # open/in-progress/blocked buckets
    │   │   └── lexical.json             # BM25, same format as memory_store
    │   ├── views/
    │   │   ├── AGENDA.md                # today + overdue + next 7 days (human-readable)
    │   │   └── TODO.md                  # open tree, grouped by root
    │   └── logs/audit.jsonl             # every mutation, append-only
    └── projects/<project_key>/          # same subtree, project-scoped work items
```

设计决策，刻意与 `memory_store` 保持镜像：

- **一条记录一个 markdown 文件。**人类可读、可 grep、可 diff、可同步（git / Syncthing / iCloud）——这是本地优先的承诺。frontmatter 是结构化的一半；正文是自由散文/清单。
- **索引与视图是派生的、可重建的缓存**，在每次作用域写入时于作用域锁下刷新（`refresh_scope_artifacts` 模式）。损坏恢复 = 从 `records/*.md` 重建，与 `rebuild_scope` 完全一样。
- **原子写入 + 审计 JSONL**，复用既有的 `atomic_fs` 辅助函数。
- **为什么不用 SQLite？**量级是人类规模（数千而非数百万），时间查询由一个小型有序索引承担，而一记录一文件让导出/备份/同步的故事与持久记忆完全一致。`LedgerStore` trait 边界保证了之后仍可引入 SQLite 后端而无需改动调用方。

## 6. 工具面：一个 `ledger` overlay 工具

一个 **overlay 工具**（与 `memory`、`scheduler` 一样——它需要存储与调度管理器），注册在 `crates/app/bamboo-server/src/app_state/tools.rs`，并列入 `SERVER_TOOL_NAMES`。单个工具、按 action 分发，形状模仿 `memory` 工具，以便模型迁移已有习惯：

| Action | 用途 |
|---|---|
| `upsert` | 创建/更新记录（kind、标题、时间、优先级、parent……） |
| `transition` | `done` / `cancel` / `block` / `reopen`——同时核对关联的调度 |
| `query` | 按时间窗（"周五前到期"）、状态、kind、标签、作用域查询；日程快捷方式 |
| `get` | 完整记录 + 子记录 |
| `decompose` | 一次调用在父记录之下创建子记录 |
| `promote` | 把当前会话 `TaskList` 中的条目提升进台账 |

系统提示（与 `memory` 工具指引所在的位置相同）会教给模型：*当用户陈述一个承诺、截止时间或事件——写进台账；当被问"我手上有什么"——查询它；绝不要把用户承诺只留在会话任务列表里。*

## 7. 调度桥：真正会触发的提醒

到这里，台账就不再只是一个数据库，而成了一位助理。

1. 在 `crates/core/bamboo-domain/src/schedule/domain.rs` 与 `NativeTriggerEngine` 中**新增 `ScheduleTrigger::Once { at: DateTime<Utc> }`**（`next_after` 返回一次 `at`，之后返回 `None`；存储在该调度的终态运行后将其停用）。一个小而独立可用的改动。
2. **由台账托管的调度。**当一条记录获得 `remind_at`/`recurrence`，`LedgerStore`（经由服务端实现的 `ScheduleBridge` trait）upsert 带有 `run_config` 元数据 `ledger_record_id` 标记的 `ScheduleSpec`，并把 id 回存到 `record.schedule_ids`。把记录迁移到 `Done`/`Cancelled` 会删除/停用其调度——不变式是*调度存储绝不比意图活得更久*。
3. **当提醒触发时**，既有的管理器路径已经做好了一切：创建会话，`task_message` = "Reminder for ledger record `<id>`: <title>. Check status, gather anything helpful, notify the user."。配上 `auto_execute`，无头运行会解析上下文（该记录、相关记忆），通知中继推送到桌面/ntfy/bark。因此提醒不是一个哑 ping——而是一次手握记录的代理回合（它可以检查那件事是否已经办完、起草邮件、总结需要什么）。
4. **每日日程调度**（"每天早上 08:00，查询今天的日程并发一份简报"）以默认关闭的模板交付——它只是叠在上面之上的纯配置，而这正是本设计的要点。

## 8. 提示注入：日程层

为 `external_memory.rs` 扩展一个新层，插在会话笔记与相关持久记忆之间：

```
## External Memory (Persistent)
  [observed state]
  [session memory note]
  [📅 Agenda]            ← NEW: overdue + due-today + next-48h events + top open todos
  [relevant durable memories]
  [project index / dream summaries]
```

- 从 `indexes/by_time.json` + `by_status.json` 渲染——**零 LLM 开销，不需要 BM25**；与其他层一样有长度上限（约 1200 字符）和条数限制（例如 10 条）。
- 位于易变块中，因此记录的增删永远打不破提示前缀缓存——与外部记忆避开 system 消息的理由相同。
- 由一个 `PromptMemoryFlags` 风格的配置开关（`ledger_agenda_injection`，当台账存在任何 open 记录时默认开启）控制。

正是这一层让助理*在对话内部保持主动*：任何会话、任何话题，都知道用户明天有一趟航班。

## 9. 后台循环：提取器 + 台账 gardener

两者都搭既有 `app_state/builder.rs` 中 spawn 点的便车，并遵循 gardener 的黄金法则——**先做确定性预筛，只在有活时才用 LLM**。

- **承诺提取器**——扩展 auto-dream 那一趟（它本来就在用后台模型遍历近期会话）。提取提示额外提议*台账候选*（"用户说必须在 8 月前续签护照"）。候选被写成 `status: Open` 记录，带 `created_by: agent` 与 `source.excerpt`，并在日程里以 `(suggested)` 的形式出现，直到用户或代理确认——自动捕获而不暗中越权。
- **台账 gardener**——`gardener.rs` 中的第四趟：
  - *过期：*时间已过的事件与陈旧的 done 记录 → `Expired`/归档（移出索引/视图，绝不删除——与记忆归档相同的可逆契约）。
  - *调度对账：*修复记录↔调度的漂移（写一半崩溃造成）。
  - *提炼：*已完成/重复出现的模式变成**持久记忆**（"每天 9 点服药"、"月度报告每月第一个周一到期"）——这是台账反哺长期记忆系统，闭环了用户问题中的直觉：前瞻记录一旦落定*就变成*回溯知识。

## 10. API + skill（其上的薄层）

- **HTTP：**`routes/agent.rs` 中的 `/api/v1/ledger` 作用域——`GET/POST /records`、`PATCH/DELETE /records/{id}`、`GET /agenda?from=&to=`——让 lotus/bodhi 能在代理所用的同一存储之上渲染真正的 todo/日历 UI。SSE 变更流事件（`ledger.record.updated`）搭既有的 `/stream` 变更流的便车。
- **Skill：**一个内置 `personal-assistant` skill（`builtin_skills/personal-assistant/`），承载*人设与工作流*——晨报格式、GTD 式分拣指引、分解启发式——其 `allowed-tools` 引用 `ledger` + `scheduler` + `memory`。行为策略放在 skill 里（可编辑、无需重编译）；能力放在工具里。这是代码库已有的混合模式。

## 11. 分阶段路线图

| 阶段 | 交付物 | 依赖 | 状态 |
|---|---|---|---|
| **1** | `bamboo-domain/src/ledger/` 类型 + `ledger_store`（记录、索引、视图、审计）+ 单元测试 | — | ✅ 完成 |
| **2** | `ledger` overlay 工具 + 系统提示指引 + 从会话 TaskList `promote` | 1 | ✅ 完成 |
| **3** | `ScheduleTrigger::Once` + 调度桥（记录↔调度生命周期） | 1 | ✅ 完成 |
| **4** | 日程提示注入层 | 1 | ✅ 完成 |
| **5** | 台账 gardener 趟（过期、对账、提炼） | 1, 3 | ✅ 完成 |
| **6** | auto-dream 中的承诺提取器 | 1, 2 | ✅ 完成 |
| **7** | HTTP API + 内置 skill + 每日简报调度模板 | 2–4 | ✅ 完成（SSE 变更流事件延期） |

阶段 1–4 是最小可爱版助理：跨会话记住承诺、回答"什么到期了"、触发提醒、主动提及日程。5–7 让它自我维护并在产品中可见。

## 12. 开放问题

1. **提取记录的确认策略**——`(suggested)` 状态，还是通知驱动的批准流？提议：先做"日程内建议"，之后再复盘。
2. **作用域默认值**——个人记录是 `Global`；项目内的会话是否应把工作项默认为 `Project` 作用域？提议：是，按 kind 区分（Todo→所在环境的作用域，Event/Reminder→Global）。
3. **外部日历同步（CalDAV/ics）**——明确不在本文范围；`LedgerStore` trait 与一记录一文件的布局为日后的导入器留了余地。
4. **工具命名**——`ledger`、`assistant` 还是 `todo`。`ledger` 更通用且与存储对应；`todo` 可能更利于模型从预训练迁移提示习惯。需要快速评测。
