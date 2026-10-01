# Durable 直系父级强制审批请求

这是 #1335 的 #1419 生产消费者切片。交互式本地 Bamboo Child 既有的 HostApprovalProxy 会转发其既有的类型化强制权限请求，以及按 Run 的规范化逻辑创建身份。Server 侧既有的 ParentAgentApprovalReviewer 会加载真实的 durable Child 和完整父链。ID、Child 出生/深度、直系父级、Root 和 Project 必须匹配；generation/creation 缺失，以及外来或重建的身份，都会在送达之前被拒绝。

在任何 reviewer 模型调用之前，reviewer 会通过 SessionMessenger 以 RespectSpecificWait 向直系父级既有的 SessionInbox 发布一条 RuntimeInstruction。其稳定 ID 包含 Child 出生、父级出生和 server 签发的 permission generation。完整类型化操作的摘要为精确重试提供绑定；同一 generation 下操作发生变更即判定冲突。信封限制为 8192 序列化字节。显示文本有界，私有浏览器资源、命令和网络资源会被脱敏；原始的 operation summaries/matchers/workspace/transport 字段不会持久化到 Inbox 中。环外 reviewer 另行接收有界的临时类型化工具/资源/操作摘要信息。真实的命令在那里保持可区分，包括 Bash 删除命令，且不会出现在 durable 审计中。私有浏览器信息、带 URI 的资源/摘要、被脱敏/缺失的字段、改变语法的显示投影以及超预算输入，都会在模型之前直接拒绝，而不是让模型去猜。PermissionRequest 没有单独的 action_details 字段；本切片使用其既有的 resource 和 operation_summary。Matchers/workspace/transport 字段绝不会进入该临时投影。普通的 worker 本地无 Human reviewer 保持不变。

送达或激活出错时，不做 reviewer 调用直接拒绝。成功准入并激活之后，会在模型评审前重新观察完整的 durable 血缘。既有 reviewer 仍只返回其既有的一次性布尔值；该记录不是权限授予、审批回执、任务指派或用户指令。后续的 #1335 源码切片会为这份规范记录增加类型化的 `ParentRequest` 投影，并在评审之前校验其确切作用域；它不会把记录变成授予。这一观察式身份检查不引入新的分布式变更围栏，也不会跨越 provider await 持锁。

原始 #1419 请求遵循既有的父级等待策略，不会打断特定的 Child/Bash 等待。送达后激活失败可能留下一条 durable 请求；精确重试会使用同一 Inbox 身份。后续的 #1438 实时结果切片会持久化一个终局决定并打断当前特定等待，详见 `live-parent-permission-outcomes.md`。冷 Child 恢复、通用提问、升级机制以及完整的 #1335 决策协议仍未完成。没有新增 HTTP 消息端点、新索引或 journal，这些切片也不关闭 #1335/#791。

准备好的验证使用真实的 FileSessionInbox 和真实的生产 reviewer，并配合一个计数 provider：发布失败时评审调用为零，成功的精确重试只留下一条 durable 记录，操作变更以及外来/缺失身份均被拒绝。native_tool_ceiling.rs 中的原生夹具在 always-ask 规则下运行实际编译的 serve/current_exe Child Write：reviewer 模型在返回 DENY 之前能看到 durable 父级请求，Write 从未发生，冷 Store 保留已获准入的请求。在协调的 Cargo 门禁完成之前，不宣称这些测试已执行。
