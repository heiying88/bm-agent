# 运维者钉选的远程 broker 激活

显式配置 `subagents.remote_placements[].broker_peer` 时，用既有的 endpoint、凭证环境引用与 CA 文件选定一个 scoped WSS broker。其封闭的父/worker mailbox 与角色元组即运维者权威；mailbox 一律严格小写拼写。broker 策略必须放行父端对所选 worker 角色的 Run/Steer/cancel 与 presence 查询，以及 worker 实际会用的回复种类。凭证缺失、CA/角色/路由非法或严格角色重复时，一律保持「已配置但不可用」。不带 `broker_peer` 的旧式 placement 仍会被选中，但保持阻断（fail closed）；它绝不打开父到 worker 的直连 WebSocket，也绝不回退到 Local。

实际的 Host/SubAgent 消费者在连接前——以及 Run 前再次——核对规范持久 Child 的出生、谱系、Project 与祖先元数据。严格链接把每个被选中的当前帧绑定到捕获的物理 AgentRef 与当前 correlation；类型化事件还须额外匹配逻辑出生、activation 与 epoch。TLS actor 客户端保持持久帧与实时 actor 帧的顺序。presence 只是连接观测，不是 HostRegistry 或持久所有权的权威。

固定的父订阅在 ActorChildRunner 整个连接/Run/清理生命周期内 single-flight；已取消的等待者不会发出 Run。运维者必须为每个 Host 分配独立的父身份。凭证私有捕获，绝不配发给常驻 worker。对外公开的 placement 只是一个远程徽标；浏览器配置隐藏整个严格拓扑，并在既有写锁下只接受与当前掩码回显的精确匹配。运维者配置保持持久；普通旧条目保持其 API 契约。

源内 fixture 覆盖真实的 WSS 拒绝/顺序测试，以及 default-current-exe 的 Host/SubAgent → 真实常驻 BambooRuntime → 录制 provider、精确 provider 计数、取消、已配置不可用场景、GET/保存/冷路由保留，以及冷态同 Child 缓存身份。Host 断连后，受控的旧 worker 会在显式 kill/wait 与替换前完成并物理 ACK 其旧 Run。过期父帧无法完成新的 correlation。这不能证明 worker 侧的 Run 防重放或自动恢复。

Required-packet、named-profile 与只读 Remote 守卫保持关闭。HostRegistry、远端持有的 activation、环境租约、防过期 worker Run、自动 resume、远端文件系统等价性，以及 #1311/#925/#791 的其余部分，都在本切片之外。编译/原生验收等待共享验证通道。
