# 紧凑 SubAgent 调用方

向模型公告的 `SubAgent` schema 含有 `intent`、`target`、`message`、保留字段 `reply_to` 与可选的 `role`。默认 intent 为 `chat`。不带 target 的 message 会在当前 Root 之下创建一个直接的持久 Child，并保留完整的任务正文。带 target 的 chat 通过既有 SessionInbox 投递纠偏输入。Runtime 继续掌管 activation 与父级等待。

只有新建的 Child 才能选择 `role`：随版本内置的默认角色是 `explorer`、`implementer` 和 `reviewer`，沿用既有的 Project → Global → builtin 目录优先级。省略时保持 `worker`；未知名称仍走既有的 legacy 标签回退。无效或重复的目录定义一律 fail closed。角色名必须显式给出，绝不从任务正文推断。continuation、inspection 或 control 调用一旦携带 `role` 即被拒绝；`role` 不能重新绑定 Child 已冻结的 profile。model/workspace/host 参数仍然隐藏。#791 的四字段示例并不构成封闭的字段数；这个可选的逻辑角色选择让已验收的具名 profile 消费者仍可被 LLM 触达。

不带 target 的 `inspect` 返回当前 Root 的观察树：按深度 4 展开，每页至多 32 个节点。树读取者只有在 Host 能从持久存储验证规范祖先链、Project、当前 activation 以及每个后代时，才能圈定某个 Child 的后代范围。隔离的 `current_exe` Worker 通过活动 Run 的只读 HostBridge 请求一个有界分页；它本地的 Child Session 永远不是树 authority 的来源。没有规范 Host 谱系存储的嵌套 Worker 一律 fail closed。传入 `message={"view":"tree","cursor":"<next_cursor>"}` 可读取下一页。cursor 绑定调用者生命周期、Project、规范谱系与被观察的树；树一旦变化就会拒绝过期的 cursor，要求重新读取第一页。每页都保持在 8 KiB 的工具结果上限之内。带 target 的 inspection 支持 overview、分页 message 预览、message 内容、最新 result 和错误指示。分页时，把包含 `view`、`cursor` 以及可选 `message_id` 的 JSON 对象作为 message 传入。历史 cursor 保留既有的已持久化前缀检查。整个序列化后的 ToolResult 限制在 8 KiB 以内；结构化物理 runtime 字段、工具参数和原始执行错误都不会进入紧凑结果。用户与 Child 撰写的 transcript 文本是内容，不是身份凭证。

`control` 接受 `cancel` 和 `retry`；retry 复跑同一个逻辑 Child，而不是另建一个。纠偏消息走带 target 的 chat。紧凑调用在 launch-owner 分类之前先做归一化，保留既有的分离 owner、准入闸门、取消补偿和 runtime 等待信号。legacy action 调用保留原有参数与输出形态。

该调用方覆盖从 Root 到直接本地持久 Child 的执行。它不解析 ParentRequests、不重新指派远端 actor，也不授权深层的生命周期变更。Root 的模型目录隐藏 `ask_agent`、`deploy_agent` 和 `cluster`；在远端 facade 完成之前，它们底层的直连兼容路由仍保持注册。tree/状态结果是观察数据，不是声明，也不是 activation 权威。Required-packet 的类型化结果选择器仍可通过兼容的 legacy inspection 路由使用。

源码测试覆盖真实 V2 存储、SessionInbox、adapter 归属检查、Child 范围与伪造谱系拒绝，以及 pre-commit 取消屏障。其中的 no-op runner 证明了 Host 侧的调用方与准入记账。它并未确立隔离原生 Worker 内的 Child 树支持。原生验收在与编译出的 host 及其 `current_exe` worker 实际对跑时另行记录。
