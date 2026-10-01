# Plan 默认 workspace（#1347）

Plan 在选择 workspace 之前先从持久存储加载其 Root 父项。既有的 `ChildSessionPort` 选择入口随后委派给该端口的 workspace 校验器。服务端适配器保留既有的 ProjectStore 所有权检查与 AppState 作用域的 WorkspaceResolver；本变更不新增 workspace 持久化或回退权威。

## 选择与校验

1. 非空的显式 Plan workspace 优先，并保持既有校验。
2. `project_default` 来源标记选择当前 Project 路径，而不是旧的默认派生 workspace 元数据。
3. 否则由持久的 `workspace_path_meta()` 选择会话 workspace。
4. 已指派 Project 的父项若没有该元数据，则选择其当前 Project 路径。
5. 只有未指派 Project 的旧版父项才可使用其旧的 `Session.workspace` 字段。

未指派且缺少 workspace 时，在创建 Child 之前即失败。已指派的父项既无规范元数据、也无已配置 Project 路径时同样失败：旧版相对路径或旧的可用目录提供不了替代的 Project 权威。无效的 Project 身份、不可用/非目录路径、外来所有权与越界（confinement）错误沿用既有校验器错误。发布缓存、进程 cwd、全局默认值或临时目录都不作为回退。显式的空/空白参数保持既有的省略选择器行为。创建 Child 之前一刻也保留校验。

## 验收证据

聚焦的服务端测试覆盖：持久元数据对比陈旧发布/旧版路径、显式覆盖、未指派旧版的兼容、当前 Project 路径的 CAS 更新、缺失/无效/外来路径，以及在任何 Child 持久化或父项等待挂起之前、已指派旧版的失效关闭行为。

`tests/plan_default_workspace.rs` 使用真实的首聊路由与同源构建的 `CARGO_BIN_EXE_bamboo` worker。首次聊天只存储 workspace 元数据；provider 发出的 Plan 不带 workspace 参数。只读规划器以有界文件名模式、不带路径调用 Glob，其下一次真实 provider 请求必须包含来自 Tool 结果的规范标记路径。夹具在放行 worker 请求之前先观察到持久的父项等待，随后观察到 Root 带着真实的 Child 结果恢复。仅凭已保存的 workspace 字段不能满足这一执行证明。

## 非目标

不新增 workspace 解析器、journal、lease 或 Project 迁移。SubAgent 与远程放置保持不变。同一逻辑 Child 的诞生/复用（#1348）、必需的 ContextPacket（#1342）、token 预算与常规力度传播（#1346）是另外的契约。本切片不添加任意 cwd 回退。
