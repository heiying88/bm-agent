# 已部署的常驻身份

生产 Root 的 `deploy_agent` 会在真实的 Host Store 中持久化一个常驻 Child Session，并在调用既有的本地、Docker 或 SSH 启动器之前初始化其 ActorDirectory 记录。它会在启动之前预留一个 ActorActivation，把物理 worker 作为不透明的放置引用，待启动器返回后再把该激活标记为 Running。返回的 `actor-…` ActorId 就是 Session.id，带有已保存的调用方 parent/root/birth/depth/Project。独立的 broker mailbox 属于内部实现。启动失败或激活启动失败会停止 worker 并让 Actor 退役。只要部署仍处于注册状态，Host 就会续订该激活；续订失败或观察到进程退出会停止物理 worker。

既有的内存部署注册表把该身份和确切的激活围栏与其物理句柄关联起来。`ask_agent` 和 stop 在解析活跃 ActorId 或其部署别名之前，会先校验调用方、当前 Host 出生/血缘、运行中的激活和 worker 放置。`ask_agent` 在收到 broker 回复后重新检查该绑定。已预留的逻辑 ID 不能变成别名；未知/已停止的 ActorId 在 broker 派发之前即告失败。既有未绑定的集群/对等路由保留其授权行为。重复的已存活别名会在启动之前被拒绝；并发发布绝不会替换胜出的句柄，并会停止落败的启动。

stop 使用既有的优雅句柄关闭，然后让 Host Actor 退役。失败的启动尝试保留一个如实的非活跃身份和已取消的激活。Store 重开保留 Session 和 Actor 历史。Server 重启会丢失物理控制映射，逻辑 ID 直接失败；租约会因无续订而过期。本切片不添加第二个持久化注册表或重启控制恢复。

延迟的常驻清理现在只让其实际启动的那个确切激活退役。条件式退役在既有的 ActorDirectory 事务下运行，包括租约过期之后，因此对同一 ActorId 的替换尝试在遭遇过期续订或 stop 时仍能存活。如果流式传输 provision spec 失败，本地和 Docker 部署器还会善后已 spawn 的 child；Docker 会在该错误路径中执行容器移除。这覆盖的是返回启动错误的情形，不覆盖 Docker 启动期间任意的调用方取消。

这将本地、Docker 和 SSH 启动器路径绑定到 Host 激活权威，但尚未把其 worker 本地历史与 Host transcript 统一。Broker 回复仍由 worker 上报，而不是 Host transcript 提交。不宣称存在围栏化的远程 transcript 写入者或 exactly-once 结果发布。聚焦夹具覆盖 Host Store 激活和失败的启动器回滚；broker 和 echo CLI 的覆盖仍是独立的集成边界。
