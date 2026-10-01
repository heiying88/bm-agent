# 会话作用域的命名 profile catalog

这个只读的宿主 catalog 从 Global 与持久 Session 的精确活动 Project 发现 schema-v1 定义。它不应用 profile、不创建 Child、不公开内置角色正文、不选择模型、不授予工具。那些执行边界仍属 #909/#1315；#912 跟踪其他发现来源。

## 权威与 API

`GET /api/v1/sessions/{session_id}/named-agent-profiles` 要求既有的宿主认证或 LocalBypass。Remote Open 与 Codex 的仅响应 token 不是 catalog 权威。查询选择器一律拒绝。宿主通过 `AppState.storage` 加载精确的 Session，判定其持久的 Project 身份，并要求 `ProjectStore.get` 的精确 manifest 处于 Active。无效、缺失、外来、已归档或不可恢复的权威以静态错误失效关闭。真正未指派 Project 的 Session 只发现 Global。

Global 来自该 AppState 的数据根目录。Project 只来自 `project_store.paths().project_home(id)`，与 workspace 配置无关。Session 缓存、请求中的 Project/路径/workspace 以及提示内容都不起作用。既有 Storage 获取保持不变。既有的 ProjectStore.get 可能创建锁文件、做迁移/规范化、隔离损坏的 manifest、恢复有效备份。成功恢复出的精确 Active 权威可被接受；catalog 本身不新增写入器、目录创建、修复或恢复。

## 一次不可变观察

同一次扫描产出公开元数据与一份私有的保留定义 map。身份为 `{name, source: global|project, project_id: null|exact_id, revision}`；revision 是原始文件的 SHA256。精确查找比较每个字段，且绝不重新打开文件。之后的文件替换改变不了旧的已选正文。这是一次观察，不是原子文件系统快照，也不是实时授权。定义不可变、不可序列化（non-Serialize），Debug 只展示安全元数据。

有效的 Project 名称会遮蔽同名 Global 名称。已知的 Project 重名会阻断该名称向 Global 的回退；无关的有效名称仍可选。匿名的无效 Project 候选或被拒绝的 Project 扫描会使整个 catalog 不可用：私有查找一无所获，任何公开行都不再可选。没有 LKG、缓存世代、别名或文件名身份。仅 Global 的匿名无效行保留既有的脱敏行为。

## 上限与平台

两层共享上限：128 个候选、1,024 个目录条目（含非 Markdown 文件）、单文件 64 KiB、提示 48 KiB、实际聚合读取 1 MiB，以及保留定义/身份加序列化元数据发布共 1 MiB。调用者只能收紧这些上限。无效读取与增长探测都计数；聚合额度耗尽即关闭整个发布，不产生部分胜者。小到不可能满足的响应预算返回静态拒绝，而非超大的元数据。公开行只含身份、安全描述、状态与静态诊断，绝不含提示、文件名/路径、路由提示或工具声明。凭据拒绝保留 v1 的原始与解码标量检查；不宣称能识别任意散文中的全部秘密。

既有的保留 FD 读取器支持 macOS/Linux，拒绝每个被配置祖先与最终文件中的符号链接。实际配置的物理根必须满足该边界；生产代码不会把不安全的根规范化成安全根。其他平台报告 UnsupportedPlatform，且无法合成活动 catalog。Windows 仍是 #1336。执行证据必须区分真实执行的 macOS 门禁与未执行的 Linux/Windows 路径。

## 验证

聚焦的真实文件夹具覆盖优先级、已知/匿名冲突、精确身份、替换、脱敏、共享的无效/候选/条目/发布预算、超大文件与祖先/最终符号链接。既有的 Global 测试保留 FD 替换与有界增长探测检查。真实的 AppState/HTTP 夹具覆盖：即使存在外来缓存仍以持久权威为准、无 workspace 前置条件、缺失/无效/已归档/外来/损坏的 Project、既有备份恢复、相互独立的宿主数据根、认证与 Codex 作用域。发现过程不新建 catalog 写入器，也不新建 agents 目录。运行时应用、远程/UI、watcher、插件优先级与 Child 供给都不在本切片内。
