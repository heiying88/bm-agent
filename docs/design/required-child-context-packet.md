# 必需的 Child ContextPacket v1（#1342）

`SubAgent.create.context_packet` 让全新的一次性任务选择完整的指令投递。不提供 context packet 时沿用 legacy 行为。packet 只绑定内容，不授予任何工具、权限、角色、生命周期或任务变更的权力。

## 输入与边界

严格的 version-1 对象必须包含 `objective`、`constraints`、`acceptance`、`non_goals`、`necessary_user_instructions` 和 `recorded_decisions`。每条 acceptance 条目和 objective 都不得为空白；其余数组可以为空。可选的 `source_user_message_ids` 与 `background_message_ids` 用于选取持久的父消息。未知字段与 null packet 会被拒绝。

| 内容 | UTF-8 字节上限 / 行为 |
| --- | --- |
| 整个 opt-in 工具输入 / packet | 32 KiB；拒绝 |
| 完整的必需文本、任务简述与所选用户文本 | 16 KiB；拒绝 |
| 每行未转义的输入行 | 2,048；必需项拒绝，可选项整条省略 |
| 转义后的六段 assignment 加上获准入的 background | 24 KiB；拒绝 |
| 可选 background | 整条计最多八条 / 总计 4 KiB；按整条省略并计数 |
| 选择器 | 必需 User ID 16 个 / 可选 ID 64 个；重复即拒绝 |

默认不选择任何历史。必需选择器会解析完整的纯文本 User 消息；多模态或工具调用来源会被拒绝。System 消息或非纯文本的可选条目会被省略。创建时会报告 background 计数；provider 装配会在诊断元数据中记录额外的整条省略。

## Host 绑定与兼容路由

在持久化/入队之前，host 会重新加载持久的父生命周期，解析各 ID 与内容 SHA256 值，对 assignment 施加边界限制，并把预检委托给与执行时相同的那个首个匹配的已注册 runner。其不可变的启动事实必须是可信的内置工厂/默认 `current_exe`、默认的 `subagent-worker` 参数、本地 BambooRuntime 以及可用的 mailbox 总线。在创建与激活之前会探测 `required_child_context_v1` 兼容性。热配置不能替代实际的 runner 事实。

未知 embeddings、自定义二进制/参数/profile、Claude/Codex、远程/调度放置、常驻与 context fork 均不受支持。这里的兼容性不是持续的 SHA 校验，也不是对管理员替换过的可执行文件做已加载镜像的密码学证明。

不可变 assignment 摘要绑定了父 ID/created_at、Child ID、所选来源 ID/摘要、完整 assignment、可选 background 及其计数，以及由 host 实际推导出的 Child TokenBudget 快照。调用方无法提供该快照。`None` 保留正常的模型默认值；最终装配遵循 host/模型中更严格的安全输入上限，但不声称推导出的预留量等于任一方的原始余量。

## Worker 与 provider 边界

必需模式的运行使用全新、不可复用的 worker。既有的 RunSpec Message 元数据承载由 host 生成的绑定；可变元数据并不构成权力授予。严格的 worker 解码会在 seeding/provider 执行之前，拒绝畸形消息、缺失或被修改的绑定/assignment 以及错误的逻辑父/Child 关系；安装阶段会应用被绑定的预算。

每一轮都会投影完整的 PromptIR 与 provider 可见的工具。只有可选 background 可以按整条 Message 省略。有损的摘要/归档/手动压缩/强制溢出路径不得丢弃必需内容，也不得调用 summarizer。Responses 续写被禁用。在 reconciliation/checkpoint/reprepare 之后，实际绑定 provider 的正文与已知的工具占用必须同时满足安全输入上限与字节预算；否则运行会在下一次 provider 调用之前失败。仅凭 `never_compress` 不能证明投递成功。

`SubAgent.update` 会在变更之前拒绝替换 assignment；请创建新的 Child。已授权的实时指导仍保持只增不减。这不是 TaskCAS，也不控制后续消息语义。不支持同一 Child 的重试恢复，自动首帧重试被禁用，既有的 worker 出生校验仍然 fail closed（#1348）。不引入任何 live/resident/远程 assignment 协议或第二份 journal。

## 验收边界

CLI fixture 运行本次构建产出的真实 `bamboo serve` 产物与默认 `current_exe` worker；仅外部 provider 是伪造的。父级来源消息/background 由 fixture 预先写入受信存储，并通过 host 冷重启加载；不测试运行期外部编辑。自然的 Child 创建/update 拒绝/运行路径会检查规范内容、Unicode、可选计数、真实的 Child 预算快照、调用与持久化重开。正常创建绑定为 `None`（无显式覆盖，沿用既有模型默认值），不会继承陈旧的已持久化 Root 预算。Tiny-budget/huge-guidance 用例在 create/run 之间显式写入受信 host 存储，作为故障 fixture 而非公开变更 API；Tiny 绑定该 Child 实际的 128-token 预算；HugeGuidance 绑定其 64K 预算并保留完整的 System 文本。两者都要求 Child-provider 调用次数为零。不预设默认大模型窗口会仅因字节数而溢出。必需模式溢出以及启动时不支持/运行期配置翻转的场景要求 Child 持久化为零。单元测试另行覆盖显式预算的捕获/安装、严格解码、被改动/缺失的绑定、可选项整条省略、后续轮次、有损路径拒绝以及最终已知占用的安全性。
