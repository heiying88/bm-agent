# Ultra Root thinking mode

Ultra 是一项独立的 Root 产品策略。它通过只读规划、收敛的子级执行以及 Root 的验证与综合，来提升整个任务的推理。每次模型调用都保留其单独解析的普通推理力度。Ultra 不是 Max 的别名，也不声称是 provider 原生预算。

## 权限与传输契约

唯一的持久选择仍是 `Session.root_orchestration_only`。公开的 `thinking_mode` 由 `root_orchestration_only_enabled()` 投影得出：`true` 对应 `ultra`，`false` 对应 `standard`。Child、带父级的 Session 或仅提示词层面的增强永远不会投影出 Ultra。已选择该模式的 legacy Root 无需迁移或被动写入即可读作 Ultra。仅走索引的列表行会省略这个仅详情暴露的字段；GET 详情会加载持久权限，并单独报告普通的 `reasoning_effort`。

- 首次聊天可以选择 `thinking_mode`。既有 Root 在聊天之前必须走可恢复的 Root 模式操作；即使选择器与当前模式相同也必须走该路径。
- 选择/恢复接受规范的 `thinking_mode`、legacy 的 `enabled`，或两者一致时同时提供。存储之前先归一化为既有的布尔请求。缺少选择器、非法/null 模式以及相互矛盾的选择器都会失败。
- 终端响应会从回执中暴露 `thinking_mode_at_completion`。被后继者栅栏的恢复则改为暴露 `current_thinking_mode`。历史已提交的回执并不描述当前选择。
- 通用 PATCH 会在任何写入之前拒绝出现 `thinking_mode` 字段，包括 null/非法值。它无法绕过模式操作。
- `reasoning_effort: "ultra"` 依然非法。全局/provider/模型角色默认值无法启用 Root 权限；产品模式永远不是 provider 参数。

不新增第二个持久化字段、证明版本、journal 或阶段协议。保留既有的出生 token、终端 epoch/回执以及派发栅栏。

## 委托执行的证据与边界

聚焦验收使用生产服务器与 Root runner、Plan 工具、先等待后入队的调度器、actor 供给以及带 `BambooRuntime` 的真实 `bamboo subagent-worker`。仅模型端点是 loopback 上脚本化的 SSE 服务器。它会在 Root 持久等待期间观察一次真实的 worker provider 请求，随后允许子级完成，并观察 Root 的恢复/综合。它还会尝试一次 Root Write，并检查真实的派发权限将其拒绝。

Root 的普通 High 是显式的。规划器使用独立模型及其自身的普通 provider 默认值（无力度覆盖）；它既不接收 Root High，也不接收产品级 Ultra。这并不能证明显式的子级力度覆盖会到达 worker：该既有传播缺口由 #1346 跟踪。

Ultra 指导复用既有工具；它不强制每个任务都设置常驻的 N-agent、Plan 或 Review 阶段门禁。必需的用户目标、约束与验收标准仍是 assignment 的一部分。既有不支持的 Plan 执行器/放置路由一律 fail closed；本切片不新增原生 Ultra、远程传播、新角色、ContextPacket 或生命周期/恢复协议。
