# Supervisor 管理发布生命周期

`SessionStoreV2::management_mutate` 在每个已启动的文件系统发布作业期间，保持其既有的 lifecycle 共享锁、Task 共享锁与排序后的精确 Session 锁。Attach 会包含目标 Session；Configure 与 Detach 只锁定已验证的 Supervisor。目标损坏或被删除时，Detach 继续使用已持久化的关系。

一个私有的 Arc 持有器拥有这些实际锁。每次 Prepared 证明、运行时 sidecar 与 Committed 证明的替换都会获得一个克隆。完整的 std 作业创建同目录下的唯一临时文件，写入并同步，执行真实替换，在支持的平台上同步目录，并在释放其克隆之前完成错误清理。调用方取消、超时以及来源 Tokio 运行时的关闭，都无法释放已启动的阻塞作业所持有的锁。Session 按排序的相反顺序释放，然后是 Task 与 lifecycle。

## 既有证明协议

三个发布阶段及其故障边界保持独立：

| 调用方/运行时丢失前已完成的阶段 | 允许的持久状态 |
| --- | --- |
| Prepared 证明 | 新的 Prepared 证明与旧运行时；运营读取拒绝。 |
| 运行时 sidecar | Prepared 证明与新运行时；运营读取拒绝。 |
| Committed 证明 | 匹配的新运行时与 Committed 证明；权威回读成功。 |

任何作业完成都不承诺下一个阶段会启动。阶段之间的取消可能留下既有的 pending 状态，该状态绝不会被修复，也不会被视为成功回执。替换之后的 I/O 或 join 错误可能留下新字节：结果属于未确认，不是回滚的证据。通过既有的权限读取器重新加载；pending 或相互矛盾的证明仍按失败关闭处理。

成功的变更会在完整的运行时投影中保留规范的当前摘要、压缩事件、model-context 状态与身份。main 会话记录/原生数据与目标文件不会被重写。管理修订/链接/墓碑检查、incarnation/出生/Project 校验以及既有的有界证明 schema 保持不变。显式指定当前修订版本的后续变更，只有在之前每个已启动作业都释放同一物理边界之后才能发布。

## 验证边界

针对性的原生夹具会在每个阶段的真实 std 替换作业内部暂停。它们使用独立打开的排他 FileExt 探针检测 lifecycle、Task 与 Session 锁，并演练预先创建的独立 Store、调用方中止、超时、整个运行时关闭、替换前/替换后错误以及真实的原始文件重开。Pending 场景断言拒绝且不修复；Committed 场景演练一次真实的后续变更，并验证其持久证明无法被覆盖。删除目标的 Detach 夹具还会在仅凭 Supervisor 权限完成吊销期间持有目标 Session 守卫。

执行证据记录实际测试的平台与确切的源码/制品标识。独立 Store 在单个 OS 进程内运行；这些不是独立的进程击杀或掉电实验。Windows 保留既有的 `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)` 辅助函数，但 Windows 上的运行时验证需要单独的平台运行。注入的替换后错误并不是真实文件系统同步失败的证明。

本切片不改变 Inbox followup 锁、默认完整保存的证明写入、Supervisor bootstrap/迁移、Task 恢复、provider/runner/缓存调用方或任何公共 API。它不新增 journal、恢复/修复协议，也不为任意外部写入者提供全局保证，更不会让整个异步管理变更在取消之后完成。
