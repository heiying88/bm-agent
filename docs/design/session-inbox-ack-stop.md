# SessionInbox ACK 失败会停止 provider 执行

ACK 失败时会保留已持久化的会话记录检查点与准入游标，但它们并不能确认 ACK 已经完成。共享的轮次前置流程会在 Project、memory、prompt 刷新与 provider 执行之前停止；对于已经持久化的输入，它仍可能发布消息事件。无论是新输入还是已永久准入的重复输入，ACK 失败都返回同一条恢复消息，不暴露内部文件系统细节。

Worker 首次投递在发送准入确认或启动 provider 之前，先使用 `Agent::admit_session_inbox_at_safe_boundary_checked`。现有的仅计数 SDK 方法保持兼容，但其部分计数无法证明 ACK 成功，不得用作执行屏障。

通过现有的准入路径重试本次激活。一个新的 store 读取器会恢复持久化游标与精确的带类型消息，移除遗留的 claim，并复用永久回执，而不会再次追加该输入。针对性的测试夹具在真实文件系统 ACK 发布前后注入错误，并断言失败时 provider 调用次数为零、冷重试后输入只有一份。

本变更不启用 owned Inbox 租约、ActorDirectory 执行、worker 释放或新的 journal，也不做任何 ExactlyOnce 或整个运行时级别的完成声明。
