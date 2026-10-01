# 不可变的 Session 上下文文件

`session_history` 为已持久化的 Root 调用者支持 `action=export_context`。它会为该 Root 树中选定的一个 session 创建小型 Markdown 文件，让调用者先读取状态，再继续读取选定的任务预览。这是一个按需的只读投影。它不会启动、修改、取消或批准任何工作，也不提供全局监督者权限。

```json
{"action":"export_context","session_id":"owned-child-123"}
```

回执包含 `schema_version`、`revision`、`source_digest`、`scope`、`manifest_path`、`status_path`、`brief_path`，以及逐文件的字节/行数与 SHA-256 哈希。`reused` 表示相同的不可变内容已经存在。把返回的绝对路径传给 `Read`，例如：

```json
{"file_path":"<returned status_path>","offset":0,"limit":12}
```

后续分段读取请保持该路径/修订不变。在两次偏移之间读取新导出的修订，可能把不同时刻的观察混在一起。Read 会限制返回给模型的内容；其实现会先读完整个有界文件，再切分出行。这并不声称是部分磁盘 I/O。

## 范围与来源

调用者来自受信任的 `ToolCtx`，调用者与目标都通过 `Storage::load_runtime_control_plane` 加载。调用者必须是拥有自身逻辑根的 Root。目标必须是该 Root，或是具有相同逻辑根的 Child，并且二者可选的有效 Project 身份必须完全匹配。Assigned 与 Unassigned 的 session 互不匹配。无效的持久身份按失败关闭处理。该动作只接受 `action` 与 `session_id`；无法提供调用者、授权或输出路径。既有的 Root 历史动作在旧版 `session_history` 身份下保持既有行为。Base 与 Child 表面为了兼容保留那个旧名称，仅有 `search_current`、`read_current` 与 `read_around`。每个表面还暴露独立的、常驻的 `session_history_current` 身份，其 schema 完全相同且仅限自身。其范围来自受信任的 `ToolCtx`；它不接受任何 Session ID 或权限覆盖，Root 宽泛的 `session_history` 覆盖层也绝不会扩大它。Base 与 Child 仍然不能列出、读取、搜索或导出另一个 Session。

主模型提示还会收到一个类型化的、Session 稳定的身份块，其中包含当前 Session ID。它被放在跨 Session 的稳定前缀之后，因此不会使不变的 prompt 缓存字节失效，也不会被送入滚动压缩摘要。该块仅用于为模型定向：工具授权仍使用受信任的运行时上下文。

导出器只请求控制面 API；它从不自行调用 `load_session` 或 `save_session`。在运行时 sidecar 有效时，SessionStoreV2 只读取该 sidecar 而不加载转写。当运行时 sidecar 缺失或损坏时，其既有兼容路径可能读取 `session.json` 然后清空消息。该存储行为保持不变：本特性保证的是导出白名单，而不是旧数据上完全不存在物理转写 I/O。

显式的来源白名单是：标题、树/Project 身份、创建/更新时间戳、标题/元数据版本、可识别的最后一次持久化 run 状态、是否存在待处理问题，以及结构化任务的标题/ID/描述/状态。未知的状态字符串渲染为 `unknown`。消息、原始元数据、系统提示、摘要、问题载荷、任务笔记/证据、凭据、工具参数与 workspace 配置都不导出。允许的标题/任务字段中的任意自由文本始终只是被观察到的数据，绝不是权限。

`status.md` 把其状态标注为最后一次持久化的观察。它不查询实时 runner 注册表，也无法证明持久化的 `running` 状态仍然有效。来源时间戳是已保存 Session 的更新时间，并不声称是导出器观察到实时 run 的时刻。

`brief.md` 是有界的结构化任务预览，不是生成的摘要，也不是完整的委托契约。截断是显式的；在依据预览行动之前，必需的指令必须从原始任务获取。

## 发布与预算

文件发布在配置的 Bamboo home 之下：

```text
coordination/session-context/v1/<root-id-sha256>/<revision>/
  status.md
  brief.md
  manifest.json
```

输出路径中不会内插任何原始 session ID。修订号对 schema 与选定的安全来源做哈希，其中包括缩略文本的校验和。清单记录范围、来源摘要/时间、文件名与内容哈希。对相同来源数据的重复导出会复用并校验既有文件。来源变更会产生新修订，而不改写之前的快照。

限制为：status 8 KiB/40 行，brief 16 KiB/120 行，manifest 8 KiB，每行 Markdown 512 个 UTF-8 字节。至多展示 32 个结构化任务。标题与任务字段会被转义、压平成单行，并在 UTF-8 边界处截短；brief 会标示被省略的内容。

发布在协作的导出器进程之间按 Root 串行化。文件先在私有的暂存目录中落盘，完整的清单最后写入，再通过目录重命名公开整个文件束。工具只在发布之后返回路径。它从不编辑规范的 Session 状态或 inbox。这是一个可重建的投影，不是新的崩溃恢复协议。

每个 Root 在其全部导出目标上合计有 **64 个快照**的硬性上限。达到上限时仍可复用既有的完整修订；新修订会返回 `context_snapshot_quota`。没有自动逐出，也不会暗中删除被引用的历史。符号链接的输出组件、被更改的不可变文件、不完整的快照以及被遗弃的发布目录都会显式失败。运维人员必须解决这类错误；工具不会修复规范状态，也不会悄悄丢弃旧视图。失败的调用至多只能移除自己尚未发布的暂存目录。

这些路径是既有 Bamboo 数据边界之下的本地文件，不是新的读取授权，也不是 OS 沙箱。导出器会拒绝其受信任配置 home 之下符号链接的输出组件；它不声称能与以同一 OS 用户身份进行任意并发文件系统访问的 actor 隔离。跨 Project 的监督者视图、受限的子代授权、订阅、实时状态、检查点与 Plan 产物需要单独的能力。
