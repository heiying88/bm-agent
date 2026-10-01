# 严格本地 Child 路由上的命名 profile

服务端真正的 Child 创建器从一次不可变的 Global/活动 Project catalog 观察中解析 `subagent_type`。Project 定义保留既有优先级；已知的冲突/无效名称或匿名无效候选都会拒绝创建。catalog/Project 不可用时绝不允许回退。只有在可用观察中确实未知的名称才走旧版角色路由。没有该服务端 catalog 生产者的 Worker 嵌入体保持既有行为。

被选中的定义在配置的全局 Child 基座之后、既有子级委派契约之前应用其精确正文。它作为宿主 Child 的 System 消息交付给实际的 worker/provider。profile 的模型提示提供绑定的 provider/model，优先于旧版角色路由、低于既有显式 `create.model` 选择；单次调用的推理力度仍是 Child 自己的常规选择。被选中的定义保持宿主私有；SubAgent schema 描述该选择但不公开其正文。显式、经认证的 System 提示检查保持既有产品行为。

已知 profile 要求既有的新鲜、默认当前 exe、本地 Bamboo worker 路由。未提供时，创建会补一个有界的必需简报包（packet）：精确的任务简报/目标与常规证据期望，可选历史为空，不虚构任何用户决策或约束。已提供的简报包保留其完整的必需字段。既有的匹配度/能力检查在持久化之前拒绝；不宣称支持远程/自定义 worker。

首个应用切片只支持 Read/Glob/Bash/Edit/Write 这五个原生名称作为声明。其他/组合名称一律拒绝。profile 的 allow/deny 只会在实际约束之内收窄：Host 拥有的 #1361 上限、配置的禁用项、继承的 Child deny、权限检查、workspace/Project 校验与类型化 readOnly。Project manifest 没有额外的工具名策略字段；本切片不发明网络/秘密授权。Explorer/reviewer 名称施加额外的只读限制，即使是 Project 覆盖也一样；名称从不证明内置出身，也不授予权限。Root 的直接编排工具围栏与其配置的委派权限是两回事。

既有的 Child 元数据存储安全的选择身份/版本号，以及冻结的 model、tools、readOnly、lifetime/Project 与组合提示摘要。它不存储单独的角色正文。后续运行校验这些观察并按当前 Host 策略收窄，而不重新打开角色文件。修改文件影响的是之后的新建，而非这个已选定的 Child。SubAgent Update 拒绝替换角色/初始指派/model/力度；普通的标题修改与追加的转向消息仍受支持。

这不是一个新的封闭存储协议或全写者围栏：经认证的通用 Session PATCH、任意的来自外部的整体保存/元数据替换以及新的重试/恢复保证都不在本消费者切片内。它不关闭全部 #909/#791 验收、不安装 #1315 默认包，也不给静态工具 schema 增加会话作用域的角色枚举。既有的安全、经认证的命名 catalog 仍是发现面。

准备好的验证使用真实文件与一个同源的 `serve`/默认当前 exe worker，并仅以脚本化的 SSE 模型响应驱动，覆盖：Project 优先级、真实 provider 请求中的私有提示/model、类型化 readOnly 与真实的 Write 拒绝/成功、Host 收窄、源重载稳定性、未知名称回退，以及 Child 持久化之前对无效/重复的拒绝。在协调的 Cargo 门禁完成之前，不宣称已有执行。
