# 自有初始输入释放

内置的本地自有零工具路由在置备之前要求 `owned_initial_input_release_v1`。其置备位要求既有的严格上下文、权威的 Child birth、实际为空的原生工具上限、权限以及 broker 路由。普通旧式执行与只读 Glob 执行保持既有契约。

在取得真实的本地 transcript/游标与永久回执之后，worker 会请求权限并在 SDK 执行前等待。封闭的 4 KiB 控制消息会绑定一个全新的 UUID nonce、逻辑 Child 的 birth/血统/Project、输入 ID/generation、激活 run 与执行 epoch。未指派 Project 的 Child 必须显式给出 Project null。该请求使用既有的 Admitted Inbox kind；链路通过既有的 Event 容器暴露其保留的带标签负载，并在 AgentEvent 处理之前拦截。回复使用类型化的 ParentFrame/Steer/RunCoord，而不是文本或批准布尔值。未知/部分取值不能变成文本 steering。

worker 会先排队其真实的权限审计。broker 会在该请求之前冲刷前面的有序事件批次。Host 要求匹配的姿态确认、当前策略/拒绝表、当前 Actor 围栏与 owner、完整的当前输入 checkpoint 回读，以及成功的精确物理 Inbox ACK。它会在返回一个受两份租约共同约束的截止时间之前复查 authority。仅有 worker 本地回执是不够的。

同一个当前 owner 可以在回复丢失后重复完全相同的 nonce/请求。Host 会重读持久的 admitted 回执并保留原始截止时间。worker 重试保留该 nonce，60 秒后超时，并观测取消/断连。错误、过期或无关的释放会使激活失败；它们绝不会变成输入。释放是一次准入决策，而不是对已进入的 provider 调用的瞬时撤销。

既有的 Local 放置引用只在可信能力探测与实际 spawn 之后记录 `owned-initial-release-v1:<physical mailbox>`。这是为未来恢复消费方保留的溯源信息，不是进程已停止的证据。缺失/旧式的放置信息无法确立历史安全性。

本切片覆盖全新自有执行上的初始类型化输入，包括已支持的第二 Run/修正与 Failed/新输入 Run。它不新增旧认领/Already 恢复、替换激活、存活输入释放、续租、远程 authority、journal 或恰好一次的 provider 效应。

聚焦的源码 fixture 使用真实的 Host Store/自有 Inbox、broker 与带录制 provider 的 BambooRuntime，覆盖：ACK 失败与重试、超过过期时间仍未释放的存活 worker、错误的 nonce/epoch、取消、当前前缀拒绝、同回执重发以及冷启动单输入回读。实际的 serve/current-exe 修正/重试 fixture 要求在 provider 入口处立即拿到 Host 回执。编译与运行结果属于共享验证通道；源码 fixture 不是执行证据。

内置的回合边界桥是一个旧式的非自有 Inbox 消费方。如果其认领失败——包括某个 Actor 已把队列升级为自有格式 3 的情况——它会向已检查的运行时路径报告未解决的准入。该路径会在 prompt/provider 执行之前停下，并为其真实 owner 保留自有认领。这是一个失败关闭的兼容边界，不是租约续期，也不是对已过期 Actor 的恢复。
