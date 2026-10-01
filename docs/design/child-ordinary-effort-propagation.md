# 本地 Child 普通推理力度

Bamboo #1346 补齐了既有的本地 `BambooRuntime` RunSpec 字段。它不引入任何传输字段或持久化协议。外部 Claude/Codex 与远程 actor 适配器保持原有行为。

SubAgent 创建时已经会在所选角色的模型偏好之前解析显式力度；Plan 则解析自身的规划器偏好。两者都把结果保存到 Child 的 `Session.reasoning_effort`。actor 适配器直接发送该字段，而不会从可能继承的父级模型引用推导力度。Root 的普通力度与产品级 Ultra 都不会为 Child 提供默认值。

每个 Bamboo worker Run 都会在激活 seeding 或 provider 执行之前，把所提供的字符串对照规范的普通枚举（`none`、`low`、`medium`、`high`、`xhigh`、`max`）进行校验。非法值（包括 `ultra`）会返回静态终态错误 `invalid RunSpec reasoning_effort`。该错误不包含任何用户提供的内容。接受的值会应用到全新的激活 Session 与实际的 ExecuteRequestBuilder 上。每个受支持的池化 worker Run 都会安装自己的力度，因此后续 Child 无法保留早前的覆盖。

显式的 `none` 表示 Disabled，且仍是一个具体的请求值。省略表示不做单次调用覆盖：沿用既有 provider 自身的默认值。当前隔离的 worker provider 配置中力度默认值未设置；因此省略力度的 Run 不会输出任何推理参数。这不会把 host 的 provider/默认配置复制进 worker。

各 provider 专属的映射与参数移除回退保持不变。已配置或已持久化的力度并不能证明 provider 请求的形态。本切片不声称支持原生 Ultra，也不声称每个 provider 都支持每一种力度。

## 验收证据

`tests/child_ordinary_effort.rs` 使用生产 Root 路由、Plan 调度器、本地 actor 适配器与同源的 `CARGO_BIN_EXE_bamboo` 子进程，仅把远程模型替换为 loopback SSE 端点。它在实际的 HTTP provider 边界处观察到 Root 的 High 与独立规划器模型的 Low。既有 Plan 的 workspace 选择器被显式提供（#1347 仍是独立工作）。

同一个测试还会供给一个真实的可复用 Bamboo worker，并以 Low、High、Disabled 和省略四种取值连接执行多个独立逻辑 Child 的连续 Run。它验证实际存活的 PID 保持不变、每次 Run 时真实传输内容都会更新、省略会清除旧的覆盖、而非法/Ultra 值不会产生任何 provider 请求。worker 单元覆盖额外验证全部六个普通值、非法规范字符串、全新持久化以及不变的 Child/Standard 身份。历史 #1345 基于默认值的证据保持不变。

对同一既有逻辑 Child 的重复激活既未被覆盖也未被修复：既有 worker 会重建其创建时间戳，而既有 V2 写入器会拒绝已变化的 Child 创建身份。既有的 None/None 温态对照用例可复现该失败。其证据被保留为一个具体的相邻问题，既不声称完整的 worker 测试套件全绿，也不在此添加创建身份的传输/恢复协议。

不包含任何新的工具/Root/任务权限、packet、启动握手、恢复 journal、远程能力系统或全 provider 回退修复。
