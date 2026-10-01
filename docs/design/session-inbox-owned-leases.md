# Owned Inbox 存储租约

本文件是 #1340 的存储基础，从运行时所有权跟踪器 #1334/#1341 中拆分而来。目前尚无任何生产引擎或 worker 使用这些 API。租约只控制队列变更；它不授权会话记录写入、provider 请求或工具调用。

## 显式选择加入

`SessionInboxPort::claim_owned(target, limit, active_run_id, request)` 会将该队列升级到格式 3，同时保留 coordinator 世代与 interrupt 世代。请求中包含一个不透明的消费者身份、受信任的调用方时间，以及一个不超过一小时的正时长。每个独立消费者都要提供新的身份。本变更不安装任何定时器、续约驱动器或自动激活机制。

现有的 `claim` / `claim_for_turn` / `ack` API 会拒绝格式 3 队列。现有生产者与 coordinator 释放操作仍受支持并保持原格式。较旧的读取器会拒绝其头部。升级不可逆；回滚二进制版本无法安全地恢复一个已租约的队列。

## 存储权限

claim、renew、reclaim 与 ACK 持有既有的 lifecycle 共享锁和 Inbox 操作锁（含其文件系统锁）。claim 会在既有队列包装器中记录 owner、单调递增的消息 epoch、过期时间、incarnation，以及该条目的有效激活策略。语义 ID、信封、投递世代与原始激活意图保持不变。后到的立即 Interrupt 消息不能提升更早的 staged 或 Respect 兄弟条目。

owned 变更 API 保留其分离式 Tokio 事务，以兼容调用方取消。此外，owned claim/renew/ACK 或检查式隔离所触及的每个已启动文件系统变更，都会持有原始的 lifecycle 共享、进程操作与 Inbox FileExt 守卫，直至其完整的同步作业结束（#1352）。owned 建立阶段也会在 Inbox mkdir/open/lock 获取期间保留已持有的 lifecycle/进程作用域；同一个 FD 会并入最终的守卫集合。守卫按相反顺序释放，任何变更作业都不会重新获取它们。

运行时关闭可能丢弃异步事务并停止后续阶段，不保证整个事务完成：ACT/INT、包装器/路径轮换以及回执/移除等相互独立的作业保持既有的部分状态恢复规则。丢失的响应属于未确认状态，由精确重试或永久回执给出结果。私有写入器保留文件 fsync、写错误清理、rename 失败临时残留以及不做父目录 fsync 的行为。Windows 沿用既有的真实替换操作，没有先删除再写入的空窗。

兼容式投递、coordinator 的 GEN/INT/ACT 发布、guidance 取消以及生产者可见的隔离扫描都使用同一个私有持有器（#1353）。该持有器不会执行 owned 格式升级：legacy 与格式 2 队列保持原格式，兼容式生产者则保留既有的格式 3。运行时准入/续约/写入器释放仍属于 #1341；本次存储变更中没有生产消费者选择加入。

普通生产者按 lifecycle → process → Inbox FileExt 的顺序获取一次。Supervisor followup 准入则改为移交其完整的原始关系持有器：lifecycle → Task → 排序后的 Supervisor/目标 Session 锁 → process → Inbox FD。每个已启动的获取/发布在其 std 作业（包括错误清理）结束前都持有这些实际守卫，释放顺序相反。Followup 从不重新获取 lifecycle，也不会用借来的 lifecycle 守卫替代其 Task 与 Session 权限。其严格关系检查仍然先于精确 ID 重试执行。

窄接口的同步 Maildir 写入器在该物理作业内运行，其格式化 JSON、按世代排序的文件名、文件 fsync 与隐藏临时文件清理均与既有写入器一致。传入的 AdmissionGate 是原子权限门，不是文件系统锁：`commit` 包住实际的 rename。取消可能留下已分配的 GEN 空洞，但不会发布任何消息或准入回执。没有任何嵌套的异步写入器或阻塞任务比持有器存活更久。其他 Mailbox 调用方保持既有的异步入口。水位线替换保留其既有的、与之不同的错误/残留语义和 Windows 真实替换语义；Maildir rename 保留既有的平台行为。两个写入器都不新增目录 fsync 或先删除再替换的操作。

精确的语义/激活意图重试仍在门控与容量检查之前；容量检查仍在 GEN 分配之前。Interrupt 权限依旧先于激活权限发布。已启动的独立作业可能在运行时关闭之后完成，而后续异步阶段从未启动。已完成的 GEN 或水位线并不能证明消息已发布。Guidance 取消会把精确的信封保留为永久墓碑；已被 claim 的条目不会被撤回。既有的部分状态恢复规则与丢失响应语义保持不变。

未过期的租约属于其 owner。同一 owner 重复 claim 会返回同一个 incarnation，且不延长其过期时间。`renew_owned` 校验当前 owner、epoch 与 incarnation，并延长过期时间且不会将其回拨。到达或超过过期时间后，新的 claim 会递增 epoch 并轮换 incarnation，即使发起请求的 owner 就是前一个 owner。epoch 耗尽时按失败关闭处理。

`ack_owned` 会精确校验当前 owner、epoch、过期时间、路径、信封、世代与策略。已过期的 claim 无法 ACK。与 legacy API 一样，调用方必须先以持久化检查点保存匹配的带类型输入；本基础层不新增原子的会话记录/worker 释放协议。永久回执绑定终态租约身份，因此精确 ACK 重试在过期/重开之后仍然有效。较旧的 incarnation 无法借用后继者的回执。续约会返回新的 token；续约前的 token 或已变更的过期时间会使当前 ACK 与终态重放同时失败，即使其他身份字段全部匹配。

## 兼容性与崩溃边界

格式 3 会写入两个既有水位线文件。先升级激活文件，并附带一个已提交的 interrupt 快照。当 interrupt 文件仍为 legacy 格式时，新读取器只使用该快照；失败的旧 Interrupt 写入器无法通过修改整数来获取权限。随后 interrupt 升级为格式 3，其旧解析器在任何写入之前就会拒绝。快照缺失或无效时按失败关闭处理。被中断的头部升级本身已经对 legacy 消费者构成围栏。本变更不引入额外的 journal 或自动降级修复。

路径轮换之前，会先以租约元数据和 `session_envelope_owned_v3` 类型原子地重写当前队列包装器。旧 v2 解码器不识别该类型，其 ACK 路径亦然——该路径从不读取头部。随后消息移动到一个包含其世代、epoch 与全新 incarnation 的路径。旧的已持有 ACK 无论在 rename 前还是 rename 后都无法移除它。如果进程在这些写入之间停止，owned 重试会完成同一 incarnation，或由过期后的 reclaim 推进它。包装器仍是唯一的持久租约记录。载荷解码在返回语义信封之前会剥离租约元数据。

租约扫描与重写使用既有的有界物理传输上限（`min(8 × max_payload_bytes + 4 KiB, 32 MiB)`）。无法容纳其租约元数据的包装器直接失败且不发布。检查受请求上限与配置的 claim 批次上限约束；它只暴露世代、epoch、过期时间、是否过期与 reclaim 计数，绝不暴露消费者身份或载荷。

## 验证边界

测试通过独立的 Store/Inbox 实例重新打开真实磁盘，并在单个 OS 进程内协调操作屏障。它们覆盖活跃互斥、续约与过期、ACK 与 reclaim、过期 ACK、终态回执、被中断的路径轮换、冻结的 v2 持有 claim ACK 行为、staged 消息以及两种策略顺序。这些不是进程击杀实验，也不是 exactly-once provider 执行的证据。运行时的排队/在途续约与写入器/worker 释放围栏属于 #1341。

针对性的原生测试夹具会暂停真实的 std 作业、中止调用方，并分别关闭其运行时。它在独立的 lifecycle/Inbox FileExt 探针与同适配器进程等待者仍被阻塞时，观察内部实现作用域的 Drop。续约与 reclaim 竞争的场景在续约之后使用一个真正已过期的后继 claim 并要求 epoch2；先 ACK 场景要求终态回执且无 epoch2；先 epoch2 的过期 ACK 不产生任何发布。建立/删除、头部升级、包装器/轮换、终态重试、隔离与失败清理复用该有界矩阵。实际执行的平台/结果必须与本源码契约分开报告；这些不是 OS 崩溃、远端一致性或 provider 执行的证明。

兼容式生产者夹具复用这些真实的双 Store 与 FileExt 探针。它会暂停获取、GEN/INT/ACT 替换、Maildir rename 与隐藏临时文件清理、取消操作，以及每个生产者可见的隔离入口。容量为 1 的测试在调用方中止或内部运行时关闭后验证独立的生产者/owned 消费者互斥、门控取消与已提交门控重放、精确语义重试、冷检查、GEN 空洞与 epoch-1 后继 claim。Followup 测试还会在 detach/作用域、Project/版本变更以及删除/重建等待已启动作业时探测 Task 与两个精确 Session 锁，并在重试时要求全新授权。在另行记录实际执行的平台/结果之前，这些夹具只是源码层面的定义。

范围是五个生产核心（Inbox 路由/持有器、真实的 Supervisor 守卫、Mailbox 及其写入器），外加一个对既有 SupervisorFollowupGuard 的私有 `v2.rs` reexport。没有新增授权、journal、租约 schema、自动格式升级、生产消费者选择加入或整段异步事务保证。Legacy claim/ACK/drain、通用 Mailbox 调用方与运行时生命周期集成都不在本持有器切片范围内。
