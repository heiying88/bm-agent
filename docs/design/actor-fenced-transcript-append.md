# 显式 Actor transcript 追加（V2 v1）

`SessionStoreV2::append_actor_transcript(ActorTranscriptAppend)` 是一个需要显式启用（opt-in）的存储操作。没有生产 runner/provider 调用方，也没有通用 `Storage` 回退。它是 #1351 的一个有界部分，并不构成 #925/#791 所要求的完整运行时所有权或恰好一次投递。

## 准入

调用方需提供既有的完整 `ActorActivationFence`、精确的 Session `expected_created_at`、完整有序的 `expected_messages`，以及完整的类型化 `expected_provider_transcript`。请从持久 snapshot 中读取这些内容。前缀不一致时返回 `PrefixConflict`；请重新加载并显式决策。ID 不能为空白、不能重复，也不能与任何持久前缀 ID 冲突，即便内容完全相同。没有自动 rebase/重放，也没有永久的操作回执。

新增的规范化消息只能是 Assistant 纯文本，可选 Commentary 或 FinalAnswer。不支持工具调用/结果、推理/签名、内容部件/OCR、元数据（包括显式 null）以及压缩/保护标志。至少需要一条新消息。历史消息保留其原有角色/字段/JSON 字节。历史消息缺少稳定的 id/created_at 时失败关闭（fail closed）；不会把默认值当作迁移手段采纳。

可选的 native 分组必须锚定到持久 lane 上已绑定的精确 family/protocol/provider-instance 边界处的一条新 Assistant 消息。全 none 的路由与旧式的自动未绑定路由都会被拒绝。只准入类型化的、源自 Provider 的 OpenAiMessage、服务端的 OpenAiToolSearchCall/OpenAiToolSearchOutput、AnthropicText、AnthropicServerToolUse/AnthropicToolSearchToolResult。既有校验器会强制检查 Model/ToolResult 作者、已完成的 discovery 配对/顺序、分组身份与条目 schema。独立的文本条目不构成 discovery 分组。不支持 Host/开发者输入、通用工具调用/结果、thinking、客户端执行、路由切换以及调用方自选的 sequence/epoch。计数器必须无溢出地前进，且整个候选必须能通过严格解码。

## 发布与不确定性

该操作会持有 lifecycle shared -> Task shared -> 精确 Session 维护这一顺序的锁，以及物理文件锁。它会严格读取既有的常规 main/runtime、初始化标记与 Actor 文件，绑定 birth/id/kind/root/parent/depth 与普通 authority、当前自身的 Project/metadata 观测、完整的存活 Reserved/Running 围栏，以及 Root 配对/证明。不支持 Supervisor。它不初始化、修复或刷新任何 authority，也不授予工具 authority。它使用自身的生存期撤销检查，而不是全祖先的生存期保证。

标准的 serde RawValue 切片会逐字节保留 main 中原有的控制面/未知键、消息前缀条目与 native 分组前缀条目。只有 messages 数组以及（在提供时的）native 分组/计数器会被修补。Runtime、证明、标记、Actor、附件、updated_at、准入、模型上下文、summary、Task、Root 模式与策略都不会被写入。返回的 Session 由确认后的 main 回读加上未变更的 runtime 投影解码而来；本 API 不执行陈旧 Root 预算清理，也不做搜索/索引/缓存刷新。

一个自有的阻塞式 std 作业会在临时写入/文件同步、替换、目录同步、回读与清理全程持有实际锁。该作业启动之后，调用方中止或 Tokio 运行时停机都无法在其终止前释放这些锁。恰在 BeforeReplace 之前、任何屏障之后，它会使用新鲜的宿主 Utc::now 检查源字节是否未变以及存活围栏。不存在任何网络/provider 等待。

`BeforePublication`/准入失败意味着本操作没有替换 main。替换之后，目录同步/回读/join 失败属于 `OutcomeUnconfirmed`：main 可能已经包含这次追加。在采取任何后续行动之前先重新加载；不要重复旧请求，也不要把不确定性解读为 transcript 未变更。

## 验收边界

测试使用真实文件、预先创建的独立 Store、FileExt 锁探针与实际的阻塞式 Before/AfterReplace 屏障，涵盖延迟租约过期、后继排序、调用方中止与整个运行时停机。同一 OS 进程内的独立 Store 用于演练文件锁；这不是 OS 级多进程击杀实验。原生平台结果会随最终关卡回执一并报告；除非单独运行，Windows 的替换实现只算源码可移植性覆盖。

旧式的 full/runtime/fallback/clear 路径保留 #1350 保护。Task 与 Management 的独立发布（#1354/#1355）仍是全局运行时启用前独立的前置条件；#1356 的启动重建另行独立验收。本 API 不为 Inbox ACK、输入、取消/provider 效应、远程放置或任意的控制面重写提供担保。
