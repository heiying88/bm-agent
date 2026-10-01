# 带作用域的 broker peers

`bamboo broker serve --peer-policy-stdin` 让单个监听器启用显式的 peer auth。需在 stdin 上提供私有的 JSON 策略。`--token` 与之互斥；该监听器会忽略 `BAMBOO_BROKER_TOKEN`。非 loopback 的数值绑定要求提供 `--cert` 与 `--key`。Worker 保留既有的 Hello 与 ProvisionSpec 传输格式；其配置好的凭据通过子进程环境变量以及 `spec.bus.token` 提供，并需对私有 WSS 证书显式信任 CA。

封闭式策略包含 `peers`，每个 peer 带有 `credential`、`host`、`mailbox`、可选的 `role`、`expires_at`（UTC 时间戳），以及由精确 `mailbox` 与 `kinds` 组成的 `destinations` 条目。可选的 `cancel` 与 `presence` 数组授予确切的目标与角色。缺少授权即拒绝访问。凭据是不透明的 32–256 字节 ASCII 可见字符串，绝不承载身份声明。标识符为 1–256 个 ASCII 字母、数字、`-`、`_` 或 `.`，但排除 `.` 与 `..`。mailbox 标识与 mailbox 选择器还额外要求小写拼写，直接拒绝文件系统的大小写别名，而不是静默归一化。上限为 64 KiB、64 个 peer 以及 16 个 destinations/cancel 目标/presence 角色。重复的字段、凭据、mailbox 主体、destinations 与 kind 授权一律以静态错误拒绝。

每一帧都会检查捕获的身份、授权与截止时间。持久 Event 与实时事件批次还会校验物理来源和 QoS。当前的订阅所有权决定严格 ACK 的准入；仅用于投递的连接与已被替换的连接无法移除 pending 记录。合法的同 peer 重连/上行仍然允许。已获准入的 I/O 可以在替换或过期之后继续完成；这不是 Actor 租约，不是即时的文件系统撤销，也不提供续租或重放保护。截止时间检查覆盖节流投递、空闲连接与出站发送。

实际的严格 CLI 使用 `<base>/scoped-peers-v1/mailboxes`；legacy CLI 与内嵌的 legacy 构造器保留 `<base>/mailboxes` 及其全局 token。不声称提供积压迁移，也不声称防御受信运维人员为根目录建立别名。这不授予逻辑 Actor 权力，不注册 Host，也不部署远程 worker。#1311、#932 与 #791 仍是独立工作。

聚焦的 fixture 演练真实的 WSS/Maildir ACK，以及实际的同源 CLI 加 BambooRuntime/provider 执行，包括独立根目录下的 legacy Ask 投递。它们依赖 openssl，绝不以 Echo 替代；只有原生测试的 host fixture 线程显式设置了 debug 栈大小。Worker 进程使用默认配置。
