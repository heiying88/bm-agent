# 文档索引

刚接触本项目？请从 [`guides/GETTING_STARTED.md`](guides/GETTING_STARTED.md) 开始。顶层 [`README.md`](../README.md) 提供项目概览、能力摘要和 CLI 命令表。

## 用户指南

面向任务，与已发布的代码库保持同步。

| 文档 | 覆盖内容 |
|---|---|
| [`guides/GETTING_STARTED.md`](guides/GETTING_STARTED.md) | 安装、配置 provider，以三种方式（CLI/HTTP/SDK）运行你的第一轮对话。 |
| [`config-reference.md`](config-reference.md) | 全部 `config.json` 键、同级配置文件（`connect.json`、`schedules.json`、`model_limits.json`）、所有 `BAMBOO_*` 环境变量、机密掩码契约、静态加密以及损坏配置恢复。 |
| [`lifecycle-hooks.md`](lifecycle-hooks.md) | 配置命令或外部多语言脚本生命周期处理器、事件负载、决策、运行时选择以及实用示例。 |
| [`guides/CONNECT.md`](guides/CONNECT.md) | 通过 Telegram、飞书/Lark 或微信个人号（iLink）操控 Bamboo。 |
| [`guides/PLUGINS.md`](guides/PLUGINS.md) | 安装/更新/移除 plugin，以及三层 URL 信任模型。 |
| [`guides/DEPLOY.md`](guides/DEPLOY.md) | 长期运行 `bamboo serve`：裸二进制、systemd、Docker、反向代理、备份。 |
| [`guides/API.md`](guides/API.md) | HTTP/SSE API 参考。 |
| [`guides/MIGRATION_GUIDE.md`](guides/MIGRATION_GUIDE.md) | 跨破坏性变更升级。 |
| [`architecture-crates.md`](architecture-crates.md) | Cargo workspace 的 crate 分层与依赖方向——在扩展 Bamboo 本身（而不只是使用它）时很有用。 |
| [`项目分析报告.md`](项目分析报告.md) | 面向二次开发的全景分析：架构分层、关键子系统、对外接口、前端体系与常见改造切入点。 |
| [`claude-code-executor.md`](claude-code-executor.md) | 将 `claude` CLI 作为子代理执行器驱动的协议参考（[配置参考](config-reference.md#sub-agents--external-cli-executors)中的 `subagents.executor = "claude_code"`）。 |
| [`codex-executor.md`](codex-executor.md) | 配置四种 Codex 认证/计费模式、隔离的 `CODEX_HOME`、自定义 provider，以及推荐的由父级按运行路由的 token 模式。 |
| [`security/KNOWN_VULNERABILITIES.md`](security/KNOWN_VULNERABILITIES.md) | 跟踪中的 `cargo audit` 公告及其解决状态。 |
| [`../examples/`](../examples/) | 可编译、可运行的代码——快速上手、流式传输真实事件类型、自定义工具、恢复 session、`ExecuteRequest` 逃生舱、连接 MCP 服务器，以及独立的公共协议 ToolEvent 记录器 plugin。在 CI 中构建，因此不会悄悄偏离真实 API。 |

## 评估证据

- [`evaluations/retrieval-window-rollout-v1.md`](evaluations/retrieval-window-rollout-v1.md)
  记录了检索窗口上下文管理的隐私安全配对聚合证据和明确的默认策略决策。它可以借助
  `bamboo-analytics` 的 `retrieval_rollout_report` 示例，从带版本的夹具复现。

## 设计说明（历史）

[`design/`](design/) 收录了在所描述工作*之前*或*期间*编写的实现计划和架构
RFC——它们提出的大部分内容如今已经发布。保留它们是为了溯源（设计背后的
“为什么”、已批准的决策、考虑过的替代方案），但它们**不随当前代码库维护**，
可能描述的是中间状态或已被取代的状态。如果设计文档与代码不一致，以代码为
准——请把文档当作历史来读，而不是规范。

<details>
<summary>完整列表</summary>

- [`design/api-v2-transport.md`](design/api-v2-transport.md) — 统一 WS 传输 + 进程内 TLS 设计（API v2 传输 epic）。
- [`design/architecture-overview.md`](design/architecture-overview.md) — broker 中介的远程子代理、轴辐式拓扑。
- [`design/ask-agent-design.md`](design/ask-agent-design.md) — 面向子代理的持久 mailbox 请求/应答（`ask_agent`）。
- [`design/edit-fuzzy-matching-design.md`](design/edit-fuzzy-matching-design.md) — `Edit` 工具的模糊匹配设计。
- [`design/ergonomic-sdk-plan.md`](design/ergonomic-sdk-plan.md) — 当前 `bamboo-sdk` 门面（`Agent`/`AgentBuilder`）背后的计划。
- [`design/feishu-adapter-plan.md`](design/feishu-adapter-plan.md) — 飞书/Lark `bamboo-connect` 适配器计划（epic #447 第 3 阶段）。
- [`design/multi-provider-routing-refactor-plan.md`](design/multi-provider-routing-refactor-plan.md) — 多 provider/多模型路由（session 级 provider+model）。
- [`design/personal-assistant-ledger.md`](design/personal-assistant-ledger.md) — ledger 记忆子系统设计。
- [`design/phase1-provider-registry-plan.md`](design/phase1-provider-registry-plan.md) / [`phase1-provider-registry-patches.md`](design/phase1-provider-registry-patches.md) — `ProviderRegistry` 多实例 provider 运行时，计划 + 补丁拆分。
- [`design/prompt-cache-architecture-plan.md`](design/prompt-cache-architecture-plan.md) — 为稳定 provider 提示缓存命中所做的 prompt/消息/缓存重组。
- [`design/provider-model-first-class-plan.md`](design/provider-model-first-class-plan.md) — 让 provider+model 成为一等公民选择单元。
- [`design/remote-actor-plan.md`](design/remote-actor-plan.md) — 远程子代理 actor 接缝（P0/P1/P2）。
- [`design/remote-mailbox-broker-design.md`](design/remote-mailbox-broker-design.md) — 独立 `bamboo broker` 网络 mailbox 设计。
- [`design/subagent-actor-runtime-design.md`](design/subagent-actor-runtime-design.md) — 子代理如今运行所依赖的虚拟 actor 模型。
- [`design/subagent-store-mailbox-interface.md`](design/subagent-store-mailbox-interface.md) — `bamboo-subagent` 的 `store/`/`mailbox/` 接口规范。
- [`design/wecom-adapter-plan.md`](design/wecom-adapter-plan.md) — 企业微信智能机器人（长连接）渠道适配器方案（后续渠道，未实施）。
- [`design/wechat-ilink-adapter-plan.md`](design/wechat-ilink-adapter-plan.md) — 微信个人号 iLink 协议适配器（首个微信通道，已实施）。
- [`design/wechat-ilink-architecture.md`](design/wechat-ilink-architecture.md) — 微信渠道架构讲解：网关、适配器、bridge 与智能体的关系及数据流。
- [`design/reviews/actor-runtime-self-review-2026-06-12.md`](design/reviews/actor-runtime-self-review-2026-06-12.md) — actor 运行时的一份自审快照。

</details>

要新增设计文档？如果它是尚未发布工作的计划/RFC，请从一开始就放进
`design/`——功能落地后，把真正面向用户的内容提升到上面的指南中，而不是让
计划文档成为唯一的文档。
