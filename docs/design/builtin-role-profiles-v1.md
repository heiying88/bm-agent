# 内置角色包 v1

Skills 包提供私有的 `explorer`、`implementer` 与独立的对抗式 `reviewer` 定义。每个提示写明其职责、有界的任务指派、停止条件与证据期望。提示上限 4 KiB（调用者可收紧），不使用模型提示，且只使用既有的 Read、Glob、Bash、Edit、Write 声明。Explorer/reviewer 允许 Read/Glob、拒绝另外三个；implementer 列出五个工具，且仍需通过 Host/Project/profile 交集。声明本身既不强制也不授予任何能力。

`ScopedNamedAgentCatalog::discover_with_builtins` 显式选择 Project > Global > Builtin 的优先级。安全的公开元数据包括来源 `builtin`、名称、描述以及该精确版本化文档的 SHA-256 版本号。旧的 `discover` 与仅 Global 的 catalog 保持不变。已知的高层冲突会遮蔽同名 builtin；匿名的无效或不可用来源会关闭选择，而不回退到 builtin。静态定义共享 catalog 的候选/文本/发布预算。

定义正文与工具策略保持私有（不可序列化、Debug 仅含元数据）。保留的身份只选择那次精确的不可变 catalog 观察，而非当前权限。生产级 SubAgent 应用、冻结的 Session 身份/重试连续性、原生只读强制、安全的 Tool schema 与经认证的 catalog 端点集成，留待稍后的合并消费者交付节点验证。仅凭这个包并不能完成那些 #1315 验收标准。既有的 Plan 及其硬只读规划器保持不变。
