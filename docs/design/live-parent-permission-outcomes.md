# 直系父级权限的实时结果

真实的 Host 会为强制的类型化权限请求创建一个不可序列化的作用域。它绑定 Child 尝试、激活 run、事件 epoch、回复 ID 和一个固定的 240 秒期限。取消、所有权丢失或 epoch 变更都会使其失效。durable 数据无法重建该权威。

规范的 server reviewer 通过 SessionInbox 以 `InterruptSpecificWait` 送达脱敏后的请求，在既有的 Parent 变更锁下提交不可变的类型化 transcript 证明，然后激活父级推理。原有的 SubAgent 子级等待仍由其既有协调器持有；重新武装会保留其注册时间戳和超时时间戳。Child 完成优先于过期的收尾 runner。

一条独立的不可变终局消息记录唯一的 Approved 或 Denied 胜者。强制询问请求现在携带类型化的 `ParentRequest` 投影，其中包含 Child 和直系 Parent 的出生、固定的 Host attempt/run/epoch/reply 作用域、确切的操作摘要、策略修订、一次性最大委托、期限以及有界的 Deny/ApproveOnce 选项。终局消息携带绑定到该确切请求的类型化 `ParentResolution`。两个投影在评审或重放之前都会与其既有的规范字段核对；持有它的直系 Parent 可以检查规范 transcript 证明，包括该 Parent 本身是 Child 的情况。它们仍是审计事实，不是可移植的授予。两条消息都限制在 8 KiB 内并受保护不被压缩。SessionRepository 完整/终局/检查点保存中既有的类型化消息保留机制，会在有界准入游标逐出请求 ID 之后仍然保留它们。不会跨越 Inbox 或 provider await 持有任何 Parent 锁。

待处理的重放不发送回复，也不发起额外的模型调用。证明缺失、冲突或未知都会拒绝；Inbox 永久回执/送达序列以及实时作用域的首次准入备忘录，可防止因记录丢失而产生新的期限。存储或中继失败属于未确认状态，不会捏造 durable 的 Denied。只有确切记录在案的胜者，才能在重新进行血缘和策略检查之后，在同一当前作用域内重试。

支持该功能需要真实的 AppState 规范 V2 存储以及相同的 repository 和锁定 store 的 Arc 装配。不支持任意的破坏性存储写入者。这不会恢复冷 Child、冒充 Human、创建宽泛的权限授予，也不会完成 #1335/#791 中完整的 ParentRequest 生命周期。
