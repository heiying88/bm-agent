# 类型化权限迁移

权限暂停仍会暴露遗留的 `question`、`options: ["Approve", "Deny"]` 和 `allow_custom` 字段。新客户端应额外读取嵌套的 `permission_request` 对象。它携带风险、原因、生效模式和允许决策的稳定 snake-case 值。

第一阶段有意只宣告 `allow_once` 和 `deny_once`。遗留的 `Approve` 会被转换为以稳定 session id 为键的一次性授权，由停靠的工具重新执行消费；它无法授权另一个 session 或之后的调用。高危与配置为始终询问的提示通过 `reason_code` 区分。显式拒绝规则先于绕过被求值，并返回拒绝而不是可覆盖的提示。

记住的 session/workspace/全局作用域、matcher-id 校验、持久规则 CRUD/CAS 以及远程策略传播仍由 #601 跟踪。在 Bamboo 将它们纳入 `allowed_decisions` 之前，客户端不得显示这些选项。
