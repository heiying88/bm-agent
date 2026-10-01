# 原生 Child 工具 ceiling v1（#1361）

`SubAgent(create, context_packet=...)` 会基于本 AppState 实际持有的 Builtin Arc 与完整的 Composite/Overlay 所有者链，在激活时自动推导出工具 ceiling，覆盖 Bash、Edit、Glob、Read、Write。Config 中被禁用的引用会先解析出确切所有者，再解析别名。外来 shadow 与未知 wrapper 无法证明原生来源。Root 的九个直接编排工具是独立的一套；Ultra 可以委托 Write。不带 context packet 的路由保持原有设置。

已分配的 Project 标识必须类型正确且一致，并在本 AppState 的 ProjectStore 中拥有精确的活跃 manifest。未分配同样可用。不存在 Project 级工具名策略字段：该字段缺省即表示没有额外的名称限制。既有的 workspace、权限与只读检查仍会收窄执行范围。既有的 ProjectStore 恢复逻辑保持不变；不声称提供网络/密钥/workspace 隔离。

封闭式启动载荷包含：version、Child/parent/root、规范的主机 birth/depth、显式且可为空的 Project 以及排序去重后的名称；上限为 16 KiB/五个名称。必需载荷缺失或为 null、未知/重复字段、不支持的版本或名称、能力扩张或 Run 不匹配，一律 fail closed。Host 与两条 fleet 派生路径都会在 provision 之前探测 `native_tool_ceiling_v1`。Worker 在构造之前校验，并在 seeding 之前校验精确的 Run 身份。严格模式的全新一次性 worker 不包含 MCP、skill 工具与自动选择、嵌套派生及组合备选。隔离的 provider 保留其最小配置槽位，不引入 Host 的工具搜索附加项。

全部七个异步入口与同步的 schema/guide/ownership/classification 视图检查的是同一个具体边界；既有的参数与权限行为保持不变。派发会在等待权限结果之前捕获真正的原生 Arc，并调用同一个 Arc。替换品无法执行；已获准入的原始 Arc 可以继续完成，而下一次替换捕获会被拒绝。该边界在每个 Run 内不可变，不是即时撤销，也不是持久的 profile 冻结。#909/#1315 的应用仍是独立工作。

聚焦的 fixture 覆盖全部入口、所有者/别名 shadow、延迟注册、真实的权限等待期替换、三种模式下的只读/资源拒绝、封闭解析，以及 seeding/provider 之前实际 worker Run 的拒绝。原生 fixture 使用同源的 CLI Host 与 current_exe worker，仅伪造远程模型，在释放 worker 之前观察持久等待，并覆盖 Ultra Root 与 Child Write 的对比、实际的 Read/Glob Config、Project 路由与 legacy 路由。在录制的 Cargo/native 门禁执行之前，这些定义仅存在于源码层面。
