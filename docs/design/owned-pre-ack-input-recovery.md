# 本地 Actor 的 pre-ACK 输入恢复

`SubAgent.run(reset_to_last_user=false)` 可以为 Ultra Root 的零工具本地 Child 恢复一条已过期且已归属的 Inbox 认领。上一次激活必须是 Failed 或已过期，并具有 Host 持久化的本地 `owned-initial-release-v1` 放置证据。其确切输入必须已被 checkpoint，且没有永久性的 Host ACK 回执。runner 缺失并不能证明旧 worker 已停止。

正常的 run 请求会重新加载当前 Child 并保留其历史。runner 获取实际的替换 Inbox 持有者，认领一次新的 Actor 尝试，并要求 Storage 对完整的当前前缀返回 `AlreadyCheckpointed` 回读。只有 worker 副本中通过校验的目标才会失去保留的 bookkeeper 键；Host Main 保持完整，普通的 Domain 匹配器也保持不变。在既有释放逻辑允许 provider 准入之前，类型化启动、当前权限姿态与实际的 Host ACK 仍必须成功。

聚焦的 fixture 包含真实的 Store/Inbox 拒绝场景，以及一个始终无法进入其 provider 的存活旧 worker。原生 fixture 使用真实的 serve、SubAgent、worker 与 Host 持久化认领：一个可读但不可写的已准入目录会导致真实的 ACK 失败，随后冷启动的 run(false) 等待未变更的物理租约过期。它会在替换的 provider 准入之前检查一次纠正、一次替换回复以及 ACK。这些 fixture 尚未运行。

已释放的输入、legacy/未知的放置来源、存活的认领、多个输入、reset、只读工具以及远程恢复仍不受支持。本切片不完成更广泛的 #791、#1055 或 #1341 验收。
