# 规范出生 census v1

`SessionStoreV2::canonical_birth_census` 是一个内部的观察性端口。它没有生产环境的实际调用方，也不是通用 Storage trait 或公开 DTO。它扫描真实的同 id Root 槽位和每个 Root 的同 id Child 槽位，既不查询索引，也不依赖调用方提供的 kind/path。所有物理候选在资格判定之前都计入；即使一个为空，两个候选也构成冲突。空、部分、仅 marker、仅 Actor、仅吊销或残留证据绝不会判为空缺。

## 支持的证据

只有一对完整、普通的 Root/Child Main/Runtime 才能产生 `PresentIdentity`。两个有界缓冲区必须在 id、kind、root、parent、depth、birth、确切的类型化 Project 和普通 authority 上一致。空 Root 的 root-id 归一化只允许在其自身的 Root 槽位上进行。完整的紧凑帧/扁平校验使用捕获的 Main 缓冲区；该端口绝不调用旧的异步加载器或 Runtime 回退。原始 Project 的优先级得以保留，而畸形的高优先级值、重复的 Project 字段和空白字符都会被拒绝。Actor 派生所做的兼容性修剪不构成权威。

Actor 记录和初始化 marker 要么双双缺失，要么是一对封闭且校验匹配的记录，包括一次独立的 Project 比较。缺失成员不会重置 Cold0/Retired0。同 id 吊销使用仅限 census 的三字段解码器，只报告记录在案的证据，不判定该出生是否仍然存活。既有的见证写入者/读取者保持不变。

Root/Supervisor 证明的存在、封存身份、管理状态、Ultra、未知 authority/残留以及不支持的文件类型均为 Unsupported。每个实际的 `save_session` Root 都会发布 Root 证明（包括 Standard），因此均为 Unsupported。带证明的父 Root 不会排除目标 Child 自身的普通候选。无证明的 Root 阳性明确是一个合成的旧版物理测试夹具，在预创建 Store 完成迁移后发布；它既不会移除真实证明，也不宣称是当前生产输出。

已知的惰性项为 `attachments/`、Root `children/`、`.search-index-revision` 和 `token-usage.jsonl`，其目录/常规文件类型必须精确匹配。不递归读取其负载。不写入或修复任何业务文件、索引、证据或私有 profile 字段。

## 有界读取与所有权

硬限制为：产出的 Root 条目 4,096 个、文件系统探测/工作访问 16,384 次、Main/Runtime 各 8 MiB、Actor/marker/吊销各 64 KiB，以及共享的实际返回字节 16 MiB。单项溢出检测可能多消耗一个字节，只计入共享上限一次。共享容量耗尽后不再发起进一步读取；未确认的 EOF 判为失败。计数器会被检查，stat 长度不能替代实际读取计量。这些是输入/工作量边界，不是内核超时截止或解析分配器限制。

该 API 获取既有的 lifecycle 共享 → Task 共享 → 精确同 id Session 维护守卫。既有的 Arc 持有者按 Session → Task → lifecycle 的顺序释放。一个完整启动的 std 文件系统作业在枚举、读取、出错和实际完成的全过程中持有自己的克隆；调用方取消或整个 Tokio 关闭都不能提前释放该克隆。不承诺取消之后获取过程和整个异步 API 还能完成。既有的锁文件机制不在无业务写入断言的范围之内。

`VacantObserved` 是持有那些守卫期间的时点观察，不是预留、创建授权，也不是某个 id 从未被使用过的证明。`PresentIdentity` 不是当前权限、活跃 Project、祖先授权、存活的租约/incarnation、Root 模式、生效 IR、模型/工具策略或 profile 绑定。既有的协作写入者就是一致性边界；任意的外部变更、retained-FD/no-follow 安全、远程路由以及新的持久化/恢复协议均不在范围内。

## 验证

聚焦夹具使用真实的规范临时文件和两个预创建的独立 Store。它们覆盖：真实 Child 保存与带证明 Root 拒绝的对照、初始化之后合成的旧版配对、过期/缺失/不正确的索引、重复/不完整的候选、严格的配对/Project/见证检查、真实的字节/探测上限与增长，以及在调用方中止和整个运行时关闭期间，使用独立 FileExt 句柄的已启动作业屏障和之后的合法写入者。源码评审不代表这些夹具已执行。Cargo 门禁需要协调的独占窗口；该 API 不启用任何生产 Actor、准入、worker、角色应用或 ContextRefs 路由。
