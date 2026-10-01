# Ordinary Root 工具权限证明（#1323）

这条 V2 存储边界保护 `root_orchestration_only` 及其单调递增的 `root_tool_authority_revision`，使 Ordinary Root 的控制面加载或常态的仅运行时保存无需读取完整会话记录。默认 Supervisor 仍会对照 main 文件检查其独立的管理权限；有界的 Supervisor 证明在 #1324 中跟踪。

## 持久化文件

每个 Root 都有 `session.json`、`runtime.json` 和一个小型 `root-tool-authority.json`。该证明记录其 schema 版本、Root ID、创建时间、权限身份、模式、修订版本，以及 `prepared` 或 `committed` 状态。它绑定的是 Root 的工具权限，而不是可变的 Task、Project 或对话数据。证明缺失、损坏、超限、处于 prepared 状态或与 sidecar 不匹配都属于恢复冲突。

在既有的跨进程每 Session 写入者锁之下，新建 Root 或选择新模式会按顺序发布这些持久状态：

1. 面向新元组的 `prepared` 证明；
2. 面向新控制面的 `runtime.json`；
3. 面向完整快照的 `session.json`；
4. 面向同一元组的 `committed` 证明。

任何中间阶段的崩溃都会使 Root 不可用。第 4 步之后、索引发布之前的失败会留下一个完整的规范 Root；普通恢复可以重建派生索引。保持同一工具元组的完整保存会保留已提交证明与既有的 sidecar 优先顺序。Root 复制、重建与 Supervisor bootstrap 会在未发布的暂存目录内写入其已提交证明，然后原子重命名。

控制面读取会检查常规的 main 文件条目，然后比较有界的 sidecar 与已提交证明。完整 Session 读取与完整保存仍会解析 main 文件，因此畸形的历史不会被悄悄覆盖。main 文件内容在之后发生损坏无法通过有界的控制面读取检测；完整读取会报告它。无索引的冷运行时修复在发布索引条目之前也会校验完整的规范配对。无索引未命中且保留了已吊销 Root 目录时，会使用吊销标记、运行时身份与已提交证明；它不会仅仅为了把已删除的 Root 报告为不存在而解析 main 会话记录。

## 一次性升级

在 Task 与复制 journal 恢复之前，启动阶段会持有既有的排他恢复门扫描 legacy Root 配对。只有当两个文件都能完整解析、物理身份一致且工具模式与修订版本完全吻合时，才会创建已提交证明。sidecar 领先于 main 可能是被中断的显式禁用，绝不会被提升。启动会在 journal 恢复之后再次扫描，并最后持久化写入全局迁移标记。之后的证明丢失无法触发自动重建。仅剩 main 的 legacy Root 会保持不可用，直到其规范权限依据独立证据得到修复。Child 回退行为保持不变。

该协议覆盖有序写入与普通崩溃，包括已提交证明仍在时旧运行时 sidecar 被还原的情形。任何单个可被替换的证明都无法检测攻击者将证明、迁移标记与 Session 文件一起回滚；那需要独立的单调信任锚。该证明也不能替代 Supervisor 管理检查。
