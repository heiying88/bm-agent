# Actor 初始化之后的默认 Session 写入者

本切片（#1350，是 #1341/#925/#791 的前置）只为默认的 V2 `save_session`、`save_runtime_state`（包括其完整保存回退）和 `clear_session` 设置围栏。它不启用生产 Actor 激活。

## 最终权威与受保护的发布

最终的 lifecycle → Task → 精确 Session 锁覆盖对 Actor 记录/初始化 marker 与实际 durable Session 出生的观察性读取。两个文件都缺失时保留旧版兼容。一对有效且匹配的 Cold attempt-zero 允许默认上下文写入。任何 current_attempt > 0 的记录在完成、失败、取消或退役后仍受保护。attempt-zero 的 Retired，以及缺一个文件/畸形/非常规文件/权威不匹配的情况同样受保护。读取绝不会初始化、修复或刷新 Actor 权威。

完整保存会比较来自实际 main 的完整有序消息、provider 原生 transcript 和 Inbox 准入，以及来自实际 runtime 的摘要、压缩事件和模型上下文。仅 runtime 的保存只比较其 sidecar 发布的内容：它丢弃的传入消息/native/准入不会导致误拒。缺失的受保护 runtime 无法从内嵌的 main 重建。相等性判定要求真实的 durable 值；历史读取的开销可能随 transcript 大小增长。既有的 Root/Task/Project/出生和 Supervisor 检查保持不变。

受保护的增量会返回 Unsupported 并直接附带 SessionAuthorityConflict，因此既有的 merge/runner 拒绝路径会抑制缓存发布。这不是 Task 重试。精确上下文的控制面保存仍独立校验。Clear 会在删除附件之前拒绝受保护/含糊的权威。

## 物理作业生命周期

每个已启动的目录准备、proof/sidecar/main/search 替换以及完整的 clear 清理/重建，都在一个持有实际锁的 std 文件系统作业中运行。各字段按先 Session、再 Task、后 lifecycle 的顺序释放。调用方中止和 Tokio 运行时关闭都不能让后继激活超越已启动的作业。Root 模式传递其既有的持有者；不添加共享公平锁重入。

完整保存仍按 Prepared → runtime → main → Committed 的顺序执行，保留故障边界。取消可以阻止后续异步阶段启动；已启动作业的所有权并不承诺整个事务完成。重命名之后出错可能留下已变更的 durable 字节而没有确认。Windows 保留既有的 MoveFileExW 替换/直写语义。

## 边界与证据

Task CAS/撤销/恢复（#1354）、SupervisorManagement（#1355）以及独立调用的启动迁移（#1356）仍是独立的写入者。围栏化追加 #1351、Inbox、附件创建、复制/删除/重建、runtime opt-in、全局缓存所有权、远程以及 exactly-once 保证均被排除在外。保留 Session 而从外部任意删除两个 Actor 文件，与旧版情形无法区分；不引入墓碑或新 journal。

聚焦测试演练真实的 durable 文件、merge 读取后的最终激活、零拒绝回调、独立的预创建 Store、真实的阻塞作业屏障、FileExt 锁探测、调用方中止和整个运行时关闭。平台执行证据在验收回执中单独记录。
