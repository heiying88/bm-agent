# 本地内置 worker 中的 Durable Child 身份（#1348）

每个受支持的本地 BambooRuntime 激活，除既有的 id、parent、Root 和类型化 Project 之外，还携带宿主规范 Child 的 `LogicalSessionIdentity.creation = { created_at, spawn_depth }`。出生保留时间戳精度。这是不可变的终身身份；它既不授予工具，也不意味着任务变更。宿主直接读取 durable Storage，拒绝元组不一致的持有中/缓存中候选，并在发送 Run 之前复查。派发的元组同时为该 worker 的事件批次设置围栏。

播种之前，worker 会校验既有 ActorSession 的身份形状，匹配预置时固化的深度，并直接读取其 strict V2 存储。既有 id 若携带另一份出生、血缘或 Project，会在 provider 派发之前被拒绝。worker 绝不采纳缓存中的出生。原有的 V2 final Child 写入者守卫在从读取到播种的边界上持续有效。

## 兼容性与缓存作用域

固化的 `child_creation_identity` 标志要求当前 Provision schema 具备显式能力 `durable_child_creation_identity_v1`。两条 spawn 路径在交付 provision 之前都会先探测。required-packet 的预创建检查同样要求该能力。该标志会划分 warm pool 的桶，而 Child id/出生不会：兼容的兄弟可以复用同一个进程。能力探测信任的是内置协议兼容性，而不是对已加载镜像的证明。此处不宣称支持 Remote/常驻/自定义实现。

默认的类型化缓存是 `<absolute host fabric_dir>/bamboo-runtime-logical-v1`。它在不同物理 worker 和初始 Child 身份之间保持不变。因此，最初为 A 预置的 worker 可以运行 B，冷启动的 B worker 也能找到 B 既有的 transcript、游标和确切的已准入回执。宿主 Run 消息仍是规范的激活快照；当宿主确认丢失时，既有的回执对账会恢复本地已准入的类型化输入。该共享命名空间会跳过物理 id 的兄弟 GC。这不构成新的缓存保留策略或独占写入者所有权策略。

显式指定的 `storage_dir` 会原样使用。旧的物理 id 目录既不被采纳也不被迁移；畸形/错误的出生不会获得新目录来掩盖冲突。更改宿主 fabric 作用域或显式目录时，不保证本地回执连续。在既有 strict 存储中重建公开 id 会直接失败。新宿主拒绝缺少该能力的旧 worker；旧池条目无法满足所需的桶。

省略 creation 的旧版发送方在该逻辑 id 不存在时仍可完成首次激活。该存储中重复出现的旧版 id 在播种/provider 之前即不受支持；worker 不会从存储状态推断出生。完全省略逻辑身份时，保留隔离的随机 id 旧版回退，但没有同一 Child 的连续性。required 模式下省略 creation 始终视为错误。required packet 保留其“仅全新、不延续”的边界，以及完整的内容/budget 校验和只读强制。

## 验收边界

`tests/child_creation_identity.rs` 使用录制式回环模型 provider 和默认存储运行实际编译出的 worker。它在同一个 PID 中演练 A→B→已 warm 的 B，再在另一个 PID 中演练冷启动的 B；同一份类型化输入、准入游标和回执持久保留且不重复。错误的出生/parent/Root/depth/Project 以及缺失必需 creation 都会拒绝，且额外 provider 请求为零，durable B 保持不变。宿主 Storage/缓存冲突、原子 wire 形状、能力 schema、池隔离和旧版仅首次行为均有聚焦的单元覆盖。夹具放大的宿主 libtest 线程不会改变原生 worker 栈。编译器 artifact/源码/profile 身份随执行证据一并记录；模型响应不等于模拟出来的运行时成功。

验收门禁显式提供 `BAMBOO_1348_LEGACY_WORKER_IMAGE`，用固定的旧镜像演练真实的 `fleet::spawn_worker`：缺少出生能力时，会在 fabric/provision/Run 和 provider 请求之前拒绝。常规 CI 可以省略这个可选的旧镜像夹具；被省略的执行不构成证据。录制的回执以哈希标识两个镜像。

手工 worker 夹具提供与宿主一致的既有只读 denylist 和权限强制，然后验证其实际的 provider 可调用目录。现有启动流程目前会丢失 durable runtime 的 `read_only` 字段（#1357）；本切片既不修复也不宣称验收只读字段的持久化。

本切片不引入远程身份迁移、任务 CAS、actor 所有权、租约、新 journal、全局 SessionRepository 缓存重键或通用 GC 协议。派发检查不能替代 #1341 为“激活已获准入之后的任意生命周期变更”规划的未来 owner/final-mutation 围栏。
