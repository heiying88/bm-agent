# Actor authority 基础（#925）

这是 #791 actor 运行时 epic 下 #925 的第一个本地实现切片。

## 持久化身份与尝试

`ActorId` 就是 `Session.id`。既有的 `session.json` 与 `runtime.json` 仍然是 transcript、parent、root、Project、深度以及 Session 生命周期的 authority。`ActorSession` 是该身份的带版本投影，此外还包含逻辑生命周期、可选的策略修订号、放置意图以及一个单调递增的尝试计数器。在接通有效策略 authority 之前，策略修订号始终缺席；绝不会用 0 伪造一个策略修订号。它会记录 Session 的 birth 时间戳，使被显式删除后以同一公开 id 重建的 Session 无法继承更早的激活租约。

`SessionStoreV2` 在 Session 旁边持久化 `actor-authority.json`。一次认领首先要证明该 Session 已持久存在，且其 main/runtime 身份对一致，然后才在缺失时持久发布 Cold authority 记录。只有在那次发布以及一个独立的持久 `actor-authority.initialized.json` 标记完成之后，才允许发布 Reserved 状态的 `ActorActivation`。标记已存在而 sidecar 缺失属于损坏状态，绝不代表可以从 attempt 0 重新开始。发生在 Cold 记录与标记之间的崩溃，只允许在该记录仍处于惰性状态时补完初始化。持有陈旧进程索引的独立 Store 会在同一把 actor 锁下扫描持久的 Session 目录树。该扫描会拒绝两个具有相同 ActorId 的物理 Session，即使不同 Store 缓存了不同的索引提示。Child 认领还会检查其直至 Root 的完整已保存父链，包括当前 Project 身份与相邻深度。每份已保存的祖先 birth 与 metadata 修订号都保留在 Child actor 记录中，因此曾被观测到的祖先重建无法让旧的激活围栏复活。当每次 Project 写入推进 `metadata_version` 时，Root Project 的 A→B→A 变更会被围栏拦截；Child 的 Project 身份在其首次持久 V2 保存（#1317）之后不可变。因此，被删除的中间 Child 不会留下可被认领的孙节点。这个首切片安全检查会在每次 authority 操作时扫描 Root 目录；未来可以用一个持久化的唯一 id 注册表取代这项开销。sidecar 不是第二个 Session 存储库，也不包含 transcript、broker 端点、凭据、PID、容器 id 或 worker mailbox。

Root Project 身份可以在 Session 创建之后才首次绑定或变更。当 actor 处于 Cold 或终态时，其 Project 投影会在下一次激活被认领之前，以更高的 authority 修订号完成持久更新。存活的激活保留认领时刻的 Project。如果 Session 在存活期间 Project 发生变更，所有 authority 操作都会返回 Project 迁移冲突，且不修改旧 sidecar 或租约。authority 还会记录最后观测到的自身与祖先 `metadata_version` 值：激活期间一旦出现两个及以上未曾观测到的修订缺口，即使 Project 此刻已匹配也会被拦截，因为 Root Project 可能已经变走又变了回来。`metadata_version` 同样覆盖标题与置顶变更，因此两次互不相关的 UI 更新也可能保守地拦截一个存活激活。V2 Child 的完整保存与运行时保存边界会在每 Session 写锁下拒绝 Project 变更，包括其他 Store 中的陈旧写者。首次完整保存还会在同一把锁下扫描物理 Root 树，以便在全局索引移动之前，拒绝在另一棵树中复用的 Child id。对于被拦截的存活激活，当前运行时还没有集成的取消/对账调用方。

这个写者防护阻止的是新的跨树 Child id 冲突。它不会修复已经包含两个同 id 物理 Child 的历史：这些 Child 的普通完整保存仍可能在路径之间移动全局索引。在这些磁盘上的 Session 得到修复之前，ActorDirectory 会拒绝该歧义 id 的激活。该防护只在 Child 首次保存时扫描 Root 树，而不是在每次高频运行时 checkpoint 时扫描。

每次 authority 操作都按既有的生命周期锁、runtime sidecar 锁、精确 Session 维护锁的顺序持锁。维护锁包含一个跨进程文件锁。每次变更都会读取当前记录，校验精确的激活围栏，然后以递增的修订号持久替换它。围栏包含 actor、activation id、attempt、run、owner 和 lease epoch。过期的 owner 由更高的 attempt 与 lease epoch 取代；陈旧的 start、checkpoint、finish 与围栏校验一律失败关闭（fail closed）。退役会以围栏拦下存活的 owner，并保留 Session 及其历史。

## 文件系统作业生命周期（#1349）

生命周期共享、Task 共享与精确 Session 写者防护由同一个 Arc 持有者拥有。每次 authority 或初始化标记替换都会把该持有者克隆进一个单独的 `spawn_blocking` 作业。该作业使用同步文件系统操作完成临时文件创建、写入、文件同步、替换、目录同步与出错清理。已启动的作业会持有全部物理锁直至终止，即使其异步调用方被中止，或其 Tokio 运行时在停机期间不再等待。Windows 保留 replace-existing/write-through 语义；原生屏障证据以实际运行测试的平台为准。

这保护的是每个已启动的文件系统作业，而不是整个异步 actor 操作。取消可能发生在 Cold 条目提交之后、其独立标记作业启动之前，也可能发生在观测刷新之后、激活 CAS 之前。既有的修复只接受没有标记的惰性 Cold 条目。从未启动的排队阻塞作业可以被取消。替换可能在目录同步出错之前提交，也可能在调用方收到确认之前提交；因此出错或调用方被取消并不能证明已经回滚。独立 Store 会在修复或后继 CAS 之前，在同一批物理锁下重新打开实际状态。

原生测试会在替换作业内部暂停，探测全部三把物理锁，中止调用方并停掉运行时，然后释放作业并重新打开实际文件。它们覆盖分离的 Cold 作业与标记作业、缺标记修复、观测刷新、后继 attempt/epoch/revision，以及替换前后的失败场景。这不会引入 journal、强制执行器或运行时调用方。

领域层的 `ActorDirectoryPort` 是运行时与存储之间的一条窄接缝。它暴露 ensure/inspect、claim/start/renew/checkpoint/finish/retire，以及精确的围栏校验。传入的时钟值必须来自可信的 host 运行时，而不是不可信的 worker 帧。

## 完整达成 #925 验收所需的后续集成

当前的 `SessionActivationRouter` 以及旧式 Session 写入/Inbox ack 调用方尚未使用这个端口。先做围栏校验、再做一次独立写入，并不构成原子的 transcript 或 Inbox 保证。下一个切片必须把精确围栏带入最终的持久 transcript checkpoint 与 Inbox ack 边界，并把每个激活入口都路由到同一个认领。只有这样，运行时才能认领唯一的 transcript 写者，并在所有路径上拒绝陈旧的事件/取消/ack。旧式 `deploy_agent` 的收敛与调度属于 #926/#927 以及独立的放置工作。

聚焦测试覆盖先 Session 后激活、重启连续性、相互竞争的独立 store owner、过期重试与陈旧围栏、退役、格式错误或不匹配的 authority、陈旧索引恢复、重复的物理 ID、孤儿后代、已观测到的祖先重建、Project 血统与重新指派、sidecar/标记缺失恢复，以及无效的状态机记录。对于在其父节点被删除并以同一 id 重建之后才首次被检查的 Child，没有更早的祖先 birth 记录；本切片按时间戳拒绝更晚出生的父节点，而在远程时钟纳入范围之前，#1318 将在 Child Session 创建记录中写入精确的持久化身绑定。
