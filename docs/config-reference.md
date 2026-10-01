# 配置参考

Bamboo 从 `${data_dir}/config.json` 读取的全部内容（`data_dir` 默认为 `${HOME}/.bamboo`，可通过 `BAMBOO_DATA_DIR` 或 `--data-dir` 覆盖），外加少数几个参与配置的同目录文件和环境变量。

不想手工编辑 JSON？`bamboo init` 会写入一份入门配置，`bamboo config set <dotted.key> <value>` 一次修改一个值（密钥字段会自动加密——参见下文的[密钥与脱敏](#secrets-and-masking)），`bamboo config [--show-secrets]` 则打印解析后的最终配置。本文档面向需要确切了解某个键的作用、或需要手工编辑该文件的场景。

**优先级：**`config.json` < 环境变量 < CLI 标志（`bamboo serve --port ...` 高于一切）。具体到 provider 选择，顺序为 `providers.<name>` / `provider_instances.<id>`（文件）→ `BAMBOO_PROVIDER` / `BAMBOO_<PROVIDER>_API_KEY`（环境变量，仅存于内存，永不持久化）→ `--provider`（CLI）。

下文每个 struct 的权威来源：除非另有说明，均为 `crates/infra/bamboo-config/src/config.rs`（`pub struct Config`，约在 1069 行）。此处的 struct 字段列表直接派生自该代码——如果两者不一致，以代码为准；请提 issue。

- [顶层结构](#top-level-shape)
- [Provider](#providers)
- [服务器](#server)
- [工具、skill 与钩子](#tools-skills-hooks)
- [LLM 流超时](#llm-stream-timeouts)
- [上下文管理](#context-management)
- [记忆 / auto-dream / gardener](#memory--auto-dream--gardener)
- [子代理 + 外部 CLI 执行器](#sub-agents--external-cli-executors)
- [MCP 服务器](#mcp-servers)
- [通知](#notifications)
- [`connect`——IM 桥接](#connect--the-im-bridge)
- [`plugin_trust`](#plugin_trust)
- [关键词脱敏](#keyword-masking)
- [权限](#permissions)
- [模型限额（`model_limits.json`）](#model-limits-model_limitsjson)
- [调度任务（`schedules.json`）](#schedules-schedulesjson)
- [环境变量](#environment-variables)
- [密钥与脱敏](#secrets-and-masking)
- [静态加密](#encryption-at-rest)
- [配置损坏恢复](#corrupt-config-recovery)

## 顶层结构

```json
{
  "provider": "anthropic",
  "providers": { "anthropic": { "api_key": "sk-ant-...", "model": "claude-sonnet-4-6" } },
  "server": { "port": 9562, "bind": "127.0.0.1" }
}
```

每个顶层键都是可选的（`#[serde(default)]`）——只含 `provider`/`providers` 的配置同样合法；下文的每个字段都会静默回退到默认值。`Config` 的完整字段列表：

| 字段 | 类型 | 说明 |
|---|---|---|
| `http_proxy` / `https_proxy` | `String` | provider HTTP 调用的出站代理 URL。 |
| `proxy_auth_credential_ref` | `Option<String>` | 指向隔离代理凭据的稳定引用（通常是 `proxy.default.auth`）；普通配置中不存储任何代理明文、密文或掩码。 |
| `provider` | `String` | 默认 provider 名称。默认 `"anthropic"`。 |
| `defaults` | `Option<DefaultsConfig>` | 按角色路由模型（`chat`/`fast`/`vision`/`planning`/……）；仅在 `features.provider_model_ref` 开启时才被参考。 |
| `providers` | `ProviderConfigs` | 旧版每类型单实例的 provider 配置。参见 [Provider](#providers)。 |
| `provider_instances` | `HashMap<String, ProviderInstanceConfig>` | 权威的 provider 配置，以稳定的路由 id 为键（例如两个不同标签下的 Anthropic key）。运行时构造、模型解析、模型发现以及子代理凭据作用域都会直接读取这些条目。 |
| `default_provider_instance` | `Option<String>` | 指定哪个已启用的 `provider_instances` 条目为默认值。缺失的 id 仅在作为指向真实旧版 provider 配置节的临时混合引用时才被接受。 |
| `server` | `ServerConfig` | HTTP 绑定地址/端口/TLS。参见[服务器](#server)。 |
| `keyword_masking` | `KeywordMaskingConfig` | 对出站请求体做密钥擦除。参见[关键词脱敏](#keyword-masking)。 |
| `anthropic_model_mapping` / `gemini_model_mapping` | `{ mappings: HashMap<String,String> }` | 将 OpenAI 风格的模型 id（例如 `"gemini-pro"`）映射为该 provider 兼容端点真实的上游模型 id。 |
| `hooks` | `HooksConfig` | 请求前置 hook；目前只有 `image_fallback`（纯文本模型的图片处理）。 |
| `tools` | `ToolsConfig` | `{ disabled: Vec<String> }`——在全局范围内从每个 session 的工具 schema 中省略的工具名。 |
| `skills` | `SkillsConfig` | `{ disabled: Vec<String> }`——全局排除在选择/加载之外的 skill id。 |
| `env_vars` | `Vec<EnvVarEntry>` | 注入到 `Bash` 工具子进程的用户管理环境变量。`secret: true` 条目仅持久化稳定的 `credential_ref`/`configured` 元数据；其值存放在隔离的凭据存储中，API 返回时以掩码形式给出。 |
| `default_work_area` | `Option<DefaultWorkAreaConfig>` | `{ path: Option<String> }`——session 未设置 workspace 时的默认值。 |
| `access_control` | `Option<AccessControlConfig>` | HTTP API/UI 的密码门禁（`password_enabled`，加盐哈希）。 |
| `features` | `FeatureFlags` | `{ provider_model_ref: bool, dynamic_model_routing: bool }`——增量发布开关，默认均关闭。 |
| `stream_timeout` | `StreamTimeoutConfig` | 相互独立的传输层、首个语义输出与中途语义输出的看门狗时限。见下文。 |
| `context_management` | `ContextManagementConfig` | 选择旧版摘要压缩，或需显式启用的精确历史检索窗口。见下文。 |
| `memory` | `Option<MemoryConfig>` | 记忆/auto-dream/gardener 相关设置。见下文。 |
| `subagents` | `SubagentsConfig` | 子代理执行以及 `claude_code` 执行器。见下文。 |
| `cluster_fabric` | `ClusterFabricConfig` | 由运维管理的远程节点，用于通过 SSH 部署 `broker-agent` worker；默认为空。SSH 密钥静态加密。 |
| `mcp`（磁盘键 `mcpServers`） | `McpConfig` | 外部工具服务器。参见 [MCP 服务器](#mcp-servers)。 |
| `notifications` | `NotificationsConfig` | 桌面/ntfy/Bark 通知渠道。见下文。 |
| `connect` | `ConnectConfig` | **实际并不存储在这里**——参见 [connect](#connect--the-im-bridge)。 |
| `plugin_trust` | `PluginTrustConfig` | plugin 安装信任策略。见下文。 |
| `extra` | `BTreeMap<String, Value>` | 兜底的 flatten 收纳区，存放尚未提升为类型化字段的键——`permissions`、`externalAgents`、`subagentRouting`、安装向导状态等都放在这里。即使对本版本 Bamboo 未知的字段也能无损往返。 |

环境变量的写入使用专门的带版本修订的 `/bamboo/env-vars` API。其 `revision` 是存放在凭据信封中的 env 域 CAS 修订号；每一次有语义的环境变更（包括元数据、公开值、排序和删除）都会使其前进一次，而真正的无操作则保持不变且不发出变更事件。`config.json` 参与同一个可恢复的清单事务，并受 hash-CAS 保护。密钥输入采用三态：省略 `value` 表示保留现有密钥，`value: ""` 显式清除，非空值则替换。掩码以及客户端发送的 `credential_ref`/`configured`/`value_encrypted` 字段会被拒绝。现有 Lotus 构建尚不会发送该修订号，也不会在仅编辑元数据时省略 `value`；更新该客户端契约已推迟到 Lotus 后续版本，不属于 Bamboo Issue #597 的一部分。

## Provider

`provider_instances` 是持久化与运行时的权威来源。旧版单实例的 `provider` / `providers` 形态仍被接受作为 serde/迁移兼容输入，最关键的一点是：旧安装可以在冷启动时被安全地物化为 provider instance：

```json
{
  "provider": "anthropic",
  "providers": {
    "anthropic": { "api_key": "sk-ant-...", "model": "claude-sonnet-4-6" }
  }
}
```

每个 provider 配置节（`OpenAIConfig` / `AnthropicConfig` / `GeminiConfig` / `CopilotConfig` / `BodhiConfig`，均位于 `config.rs`）共享以下核心形态——`api_key`（只写；以 `api_key_encrypted` 持久化，`GET` 绝不会以明文回显）、`base_url`（覆盖上游端点——自托管代理、Azure 风格部署等）、`model`、`fast_model`、`vision_model`、`reasoning_effort`、`responses_only_models: Vec<String>`（强制这些模型走 OpenAI Responses API 路径）、`request_overrides`（针对特定 provider 的按端点 HTTP 头/请求体微调），以及用于前向兼容字段的 `extra` flatten。`AnthropicConfig` 额外增加 `max_tokens` 和 `thinking_replay_always`（某些 Anthropic 兼容上游需要，例如 GLM 的 `/anthropic` 端点）。`BodhiConfig` 额外增加 `target_provider`（Bodhi 代理对外呈现为 openai/anthropic/gemini 中的哪一个）。`CopilotConfig` 则完全没有 `api_key`——它通过缓存的 OAuth token 认证（`headless_auth` 用于无头/CI 登录）。

对于来自 agent 循环的 GPT-5.6+ OpenAI Responses 请求，Bamboo 会推导一个稳定的、session 作用域的 `prompt_cache_key`，作为域分隔的 SHA-256 哈希。原始 session 标识符绝不会序列化进 provider 请求，非 agent 请求也不会获得生成的 key。该 key 只是一个缓存亲和性提示，可以提升路由到匹配前缀的概率，并不保证命中缓存。`request_overrides` 的请求体补丁在其后执行，因此运维人员可以替换生成的 key 或彻底移除 `prompt_cache_key`。

服务器在打开模块化配置时，会把可用的已选中旧版 provider 幂等地物化到 `provider_instances` 中。内置 provider 类型名成为稳定的实例 id（`openai`、`anthropic` 等），被选中的 id 成为 `default_provider_instance`，并且 Anthropic 的 `max_tokens` / `thinking_replay_always`、Copilot 的 `headless_auth`、Bodhi 的 `target_provider` 等 provider 专有字段都会保留。已有的凭据引用会被复用；明文绝不会被复制进 `providers.json`。迁移提交使用 provider 节的修订号和配置门面的可恢复事务协议。重新打开一个已完成迁移的配置在字节/修订号层面是空操作。如果 provider 或凭据权威处于降级状态、有旧版对账待处理，或确切的基础修订号已发生变化，迁移不会覆盖该状态；启动过程保留可读的最后已知良好/混合视图，并在之后某次安全打开时重试。

对于单个或多个 provider 账号，规范形态是：

```json
{
  "default_provider_instance": "work",
  "provider_instances": {
    "work": { "provider_type": "anthropic", "api_key": "sk-ant-work-...", "model": "claude-sonnet-4-6" },
    "personal": { "provider_type": "anthropic", "api_key": "sk-ant-personal-...", "enabled": true }
  }
}
```

`provider_instances` 条目拥有与旧版配置节相同的字段集，外加 `provider_type`（五种类型中的哪一种）和 `enabled`（默认 `true`）。显式的实例 id 总是优先于同名的旧版别名——即使该实例已被禁用或无效也是如此；Bamboo 绝不会静默复活过期的别名。临时混合状态只有在同名的真实配置节仍然存在时，才可能保留旧版默认 id。其他实例原生的路径不会把实例重新投射回全局旧版槽位。

`BAMBOO_PROVIDER` 可以选择一个精确的实例 id。传入内置类型值时，会选择该类型按字典序首个已启用的实例，从而让多账号启动具有确定性。`BAMBOO_OPENAI_API_KEY`、`BAMBOO_ANTHROPIC_API_KEY` 和 `BAMBOO_GEMINI_API_KEY` 只注入标记为接受标准环境变量覆盖的实例，不会重建旧版槽位。由旧版物化的实例保留该绑定，因此历史优先级保持稳定：环境变量存在时优先，迁移后的凭据引用在变量移除后作为回退。持久化的标记只记录绑定关系，绝不记录密钥。没有存储回退的纯环境变量实例在该变量缺失时报告 `configured: false` 和 `source: "environment"`。

设置请使用带版本修订的 `GET/PUT /v1/bamboo/config/provider-settings` 契约或 provider 实例的 CRUD 端点，实时模型发现请使用 `POST /v1/bamboo/provider-catalog/fetch-models`。旧版 `GET/POST /v1/bamboo/settings/provider` 和 `POST /v1/bamboo/settings/provider/models` 路由已不再注册；旧客户端会收到 `404`，必须迁移到规范契约。这次 HTTP 下线并不会移除上文描述的磁盘兼容输入。

## 服务器

```json
{ "server": { "port": 9562, "bind": "127.0.0.1", "workers": 10 } }
```

`port`（默认 `9562`）、`bind`（默认 `127.0.0.1`）、`static_dir`（从自定义路径提供内置前端）、`workers`（Actix worker 线程数，默认 `10`）、`tls: Option<TlsConfig>`（手动 TLS 终止的 `cert_file`/`key_file` PEM 路径——不支持 ACME/自动证书）。以上均可在每次调用时通过 `bamboo serve --port/--bind/--workers` 覆盖。

## 工具、skill 与钩子

- `tools.disabled: Vec<String>`——在全局范围内对每个 session 的工具 schema 隐藏的工具名（例如 `"Bash"`）。与之相对，SDK 的按 agent 设置 `AgentBuilder::tools([...])` 只把选择范围限定在单个进程内 `Agent`。
- `skills.disabled: Vec<String>`——全局排除在选择/加载之外的 skill id。
- `hooks.image_fallback`——当生效的模型/路径为纯文本时如何处理图片部分（丢弃、OCR 替换等——参见 `ImageFallbackHookConfig`）。`ViewImage` 默认返回 base64 多模态图片；在 `vision` 模式下启用该 hook 后，Bamboo 会在下一轮模型调用前先请求配置的视觉模型（或其配置的回退）给出文字描述。
- `lifecycle_hooks`——由配置驱动的命令或外部 `.js`/`.py`/`.sh`/`.ps1`/`.bat` 脚本处理器，覆盖 session、prompt、工具、压缩和通知事件。它存放在 `hooks.json` 中；运行时选择、安全模型以及输入/输出契约请参见[生命周期钩子指南](lifecycle-hooks.md)。

## LLM 流超时

```json
{
  "stream_timeout": {
    "transport_idle_timeout_secs": 120,
    "first_semantic_timeout_secs": 600,
    "semantic_idle_timeout_secs": 600
  }
}
```

这三个看门狗测量不同的信号，并以完全相同的方式作用于主响应流和辅助的静默模型调用：

| 字段 | 默认值 | 含义 |
|---|---:|---|
| `transport_idle_timeout_secs` | `120` | provider 调用建立其响应流的最长时间，以及随后成功接收到的非空响应体分块之间的最大间隔。SSE ping/生命周期事件、注释心跳和残缺的事件片段即使不含任何 token 也计入在内。 |
| `first_semantic_timeout_secs` | `600` | 从请求发出到第一个文本、推理或工具调用增量的最长时间。传输层保活不会延长它。 |
| `semantic_idle_timeout_secs` | `600` | 输出开始后语义进展之间的最大间隔。传输层保活不会延长它。 |

每个值都必须在 `1` 到 `86400` 秒之间。非法的持久化值会在配置加载时被拒绝；嵌入方构造出的非法值会被替换为安全默认值。超时错误会报告超时的阶段、截止时间、provider/模型标识以及最后一次传输/语义活动，但绝不包含 prompt 或原始 provider 载荷。一旦文本、推理或工具调用输出已经开始，流超时就不会重试，因为重放可能导致外部可见状态重复。主响应流在任何语义输出出现之前的超时会被标记为可安全重试，并可使用 agent 循环既有的有界轮次重试策略。辅助模型调用受同样的看门狗约束，但绝不会重放其所属的 agent 轮次。

### OpenAI 兼容代理心跳

OpenAI 兼容代理应在 `transport_idle_timeout_secs` 之内建立响应流，然后以快于该截止时间的频率转发上游响应字节或发出 SSE 心跳。Bamboo 在解析 SSE 之前，会把任何成功接收到的非空请求体分块视为传输活动，包括标准注释形式 `: keep-alive\n\n`。这些内部活动标记不会变成模型输出，也不会延长任何一个语义截止时间。

例如，CLIProxyAPI 支持周期性流式心跳，配置如下：

```yaml
streaming:
  keepalive-seconds: 15
```

CLIProxyAPI 文档标注默认值为 `0`（禁用）。使用它或其他兼容代理的运维人员应选择明显低于传输超时的心跳间隔。如果代理无法在上游长时间推理间隙内发出心跳，请改为把 `transport_idle_timeout_secs` 配置得至少与预期的语义等待一样高；不建议禁用有界的传输层看门狗。

## 上下文管理

`context_management` 默认为 `{"strategy":"summary"}`。因此，缺失配置时保持既有的模型生成会话摘要行为。另一种 `retrieval_window` 策略会把精确的原始消息保留在 Session 存储中，仅从当前活跃的 provider 窗口移除符合条件的较早完整轮次，并在模型需要更早的证据时引导它使用 `session_history_current`。

```json
{
  "context_management": {
    "strategy": "retrieval_window",
    "retrieval_window": {
      "min_recent_user_turns": 3,
      "trigger_usage_ratio": 0.8,
      "target_usage_ratio": 0.6,
      "history_tool_required": true,
      "fallback_strategy": "none"
    }
  }
}
```

| 字段 | 默认值 | 作用 |
|---|---|---|
| `strategy` | `summary` | `summary` 保留现有的摘要器路径；`retrieval_window` 在常规的轮前压力边界启用精确历史归档。 |
| `retrieval_window.min_recent_user_turns` | `3` | 保持活跃的最新完整用户锚定轮次的最少数量。必须大于零。 |
| `retrieval_window.trigger_usage_ratio` | `0.80` | 触发自动归档的 provider 已准备输入占比。 |
| `retrieval_window.target_usage_ratio` | `0.60` | 归档后的目标占比。校验要求 `0.01 <= target < trigger <= 1`，因为规划器使用整数百分比。 |
| `retrieval_window.history_tool_required` | `true` | 本版本中必须保持 `true`。若生效的可调用工具目录不包含 `session_history_current`，Bamboo 会在归档前直接失败。 |
| `retrieval_window.fallback_strategy` | `none` | `none` 表示检索归档不可用时直接失败（fail closed）；`summary` 则显式启用旧版摘要器回退。 |

检索窗口的提交需要运行时持久化，并在发布归档后状态或发送 provider 请求之前先建立检查点。该检查点在持有 Session 写锁的情况下比对归档前的精确基准；一旦出现并发的持久化转录变更，就不会写入归档，而是让引擎重新定基、重新记账、重新规划，并在有界重试之前重建 provider 请求。为已存在会话摘要的 Session 选择 `retrieval_window` 且 `fallback_strategy: "none"` 会被立即拒绝；请开启新的 Session，或显式保留摘要回退。这第一个运行时切片支持自动的轮前归档、通过无参数 `archive_context` 工具进行的模型显式请求归档，以及通过同一无摘要边界的临界溢出恢复。`compact_context` 仍然是摘要专用的控制项：在 `retrieval_window` 之下，除非 `fallback_strategy: "summary"` 显式启用旧版摘要器，否则它会直接失败（fail closed）。已经达到或低于目标占比的手动请求会被持久化地作为空操作消费掉，从而不会在重启后循环。失败的归档检查点仍可重试，且绝不会发布部分归档的状态。候选适配会精确投射边界之后的 provider 请求。被边界重置的 provider 原生推理/工具搜索重放以及先前的模型上下文账目字节都会计入触发压力，但在计算保留目标时会被一次性回收（不会被错误归类为永久固定的 prompt 成本）。原始 Session 转录始终是权威。记忆是选择性的上下文，不能替代精确历史。

## 记忆 / auto-dream / gardener

键 `memory`（`Option<MemoryConfig>`——缺省时下文所有默认值生效）。所有 dream/gardener 开关默认**开启**；下表列出的是出厂默认值，因此一个空的 `{}` 已经足够合理：

| 字段 | 默认值 | 作用 |
|---|---|---|
| `background_model` | `None` | 用于记忆提取/整理后台工作的模型；回退到主模型。 |
| `summary_target_ratio` | `0.20` | 期望的持久化会话摘要相对于其代表的原始来源 token 的大小。分层归约器保留这一全局比例，而不是在每一层重复应用。 |
| `summary_safe_window_percent` | `80` | 每个完整渲染的 map/reduce 请求占用的摘要模型总上下文窗口的最大比例，包括请求的输出和分词器安全余量。 |
| `auto_dream_enabled` | `true` | 在 session 运行过程中把会话片段蒸馏为候选记忆和笔记本条目。 |
| `auto_dream_interval_secs` | `1800` | dream 遍的运行频率。 |
| `project_prompt_injection` | `true` | 将相关的项目作用域记忆注入系统 prompt。 |
| `relevant_recall` | `true` | 为当前轮次检索相关的持久记忆。 |
| `relevant_recall_rerank` | `false` | 注入前对召回的记忆重排序（额外一次模型调用）。 |
| `project_first_dream` | `true` | 在 session 的第一次 dream 遍中优先使用项目作用域记忆。 |
| `ledger_agenda_injection` / `ledger_gardener_enabled` / `ledger_distillation_enabled` | `true` | 个人助理账本子系统的开关。 |
| `ledger_gardener_interval_secs` | `21600` (6h) | 账本 gardener 的运行周期。 |
| `gardener_enabled` | `true` | 拆分"多主题大杂烩"型记忆的后台任务；**当确定性预筛没有找到候选时，不会调用任何 LLM。** |
| `gardener_interval_secs` | `86400` (daily) | gardener 的运行周期。 |
| `gardener_volume_trigger` | `25` | 一旦累积了这么多新记忆就提前运行，而不必等待周期到来。 |
| `gardener_max_splits_per_run` / `gardener_min_sections` | `8` / `5` | 单次 gardener 遍的成本护栏。 |
| `dedup_gardener_enabled` | `true` | 后台的近似重复记忆合并遍。 |
| `dedup_gardener_min_score` | `0.6` | 触发合并的 Jaccard 相似度阈值。 |
| `dedup_gardener_max_merges_per_run` | `8` | 单次遍的上限。 |
| `memory_active_capacity` | `0` (unbounded/off) | "活跃"记忆数量的上限，超过后较旧的记忆转入归档。 |
| `capacity_max_archivals_per_run` | `50` | 单次容量执行遍的上限。 |
| `granularity_freshness_gardener_enabled` | `true` | 后台的过期/粒度整理遍。 |

自动会话压缩总是先对有界的来源分块做 map，再对其摘要做 reduce，即使选中的来源本来可以装进一次模型请求。大型终端结果会被归约为多个有界的分段；持久化的摘要保持单一的整体 `summary_target_ratio` 预算。

`project_prompt_injection` / `relevant_recall` / `relevant_recall_rerank` / `project_first_dream` 也可以通过环境变量切换——参见[环境变量](#environment-variables)——这便于在不改动 `config.json` 的情况下做一次性的容器运行。

`auto_dream_enabled`/`gardener_enabled` 开启时会刻意消耗模型 token；若要最低成本部署，请将其关闭（`{"memory": {"auto_dream_enabled": false}}`）。

## 子代理 + 外部 CLI 执行器

键 `subagents`（`SubagentsConfig`）。子代理始终作为独立的 actor 子进程运行（崩溃隔离 + 真正的并行）——不存在进程内运行时开关。

| 字段 | 用途 |
|---|---|
| `max_concurrent` | 同时运行的子代理数量上限（默认：200）。 |
| `worker_bin` / `worker_args` | 覆盖子代理 worker 的二进制文件/参数（默认为当前 `bamboo` 二进制的 `subagent-worker` 模式）。 |
| `fabric_dir` | actor fabric 的 mailbox/状态文件所在位置。 |
| `executor` | 由哪个执行器派生子进程：`"echo"`（测试桩）\| `"bamboo_runtime"`（默认——完整的嵌套 Bamboo agent 循环）\| `"claude_code"` \| `"codex"`。 |
| `claude_code_binary` | `claude` 二进制路径；`None` 时通过 `PATH` 解析 `claude`。 |
| `claude_code_model` | 传递给 `claude` 的 `--model`。 |
| `claude_code_permission_mode` | 传递给 `claude` 的 `--permission-mode`（一旦选中该执行器，即使值为 `"default"` 也总是显式传递）。 |
| `claude_code_inherit_user_config` | `false`/未设置时会加上 `--strict-mcp-config --setting-sources project`，让子进程与你的个人 `claude` 配置隔离。 |
| `claude_code_forward_env` | 在固定白名单（`HOME`/`PATH`/`SHELL`/`TERM`/`LANG`/`LC_*`/`TMPDIR`/`USER`/`LOGNAME`）之外，额外逐字转发到子进程的环境变量**名称**。 |
| `codex_binary` / `codex_model` | Codex 可执行文件以及可选的 `--model` 覆盖。 |
| `codex_mode` | `"exec"`（默认，每次激活一个进程）或 `"app_server"`（长驻 JSON-RPC，带父级审批中继）。缺少 app-server 能力时会明确失败，绝不降级。 |
| `codex_auth_mode` | `"inherit"` \| `"api_key"` \| `"custom"` \| `"bamboo"`；未设置时默认为推荐的 `"bamboo"` 父 provider 模式。 |
| `codex_base_url` | 仅在 `custom` 模式下使用的绝对 HTTP(S) URL；凭据、查询参数和片段会被拒绝。 |
| `codex_wire_api` | `"responses"`（受支持的 Codex CLI 版本唯一接受的协议）。 |
| `codex_provider_key_ref` | 仅在 `custom` 模式下使用的既有 Bamboo provider 凭据引用；密钥通过环境变量注入，不会写入生成的 Codex 配置。 |
| `codex_forward_env` | `env_clear()` 之后的额外环境变量名称；`api_key` 模式要求显式的 `OPENAI_API_KEY`，其他模式则拒绝它。`CODEX_*` 和 Bamboo 管理的 provider 密钥变量是保留的。 |
| `codex_sandbox` | 可选的显式 `"read-only"` \| `"workspace-write"` \| `"danger-full-access"`。未设置时根据子代理 profile 和父级当前 bypass 状态推导安全值。 |
| `codex_approval_policy` | exec 模式接受可选的 `"never"` \| `"on-failure"`；app-server 模式接受未设置或 `"on-request"`。跨模式组合会被拒绝。 |
| `codex_network_access` | 在 `workspace-write` 内启用网络访问；与显式 `read-only` 沙箱不兼容。 |
| `codex_allow_danger_bypass` | 禁用 OS 沙箱的第二道闸门。当前父级也必须处于 bypass 模式；root worker 总是降级并发出警告。 |
| `remote_placements` / `schedulable_placements` | 子代理可以在哪里运行（本地/某个具名的 Cluster Fabric 节点），以及调度是否可以指向它。 |
| `mcp_role_allowlist` | 限制某个子代理角色可以看到哪些 MCP 服务器。 |

```json
{
  "subagents": {
    "executor": "claude_code",
    "claude_code_model": "claude-sonnet-4-6",
    "claude_code_permission_mode": "acceptEdits",
    "claude_code_forward_env": ["MY_TOOL_TOKEN"]
  }
}
```

当你需要为不同的具名 agent 配置不同的外部 CLI 执行器设置，而不是使用同一个全局默认值时，同一套 `claude_code_*` 和 `codex_*` 字段可以按 agent 重复出现在 `ExternalAgentProfile`（`Config.extra["externalAgents"]`，`bamboo-engine/src/external_agents/config.rs`）之下。

具体的派生实现位于 `src/claude_code_executor.rs`（`ClaudeCodeExecutor`）：它在完全经过 `env_clear()` 的子进程（只透传上述白名单）中运行 `claude --output-format stream-json --input-format stream-json --permission-prompt-tool stdio --replay-user-messages --verbose [--model ...] [--permission-mode ...] [--resume <id>]`——session id 通过每个子代理 workspace 下的一个小型 `claude-code-session.json` 状态文件映射到 `claude` 自己的 `--resume`。

Codex 的配置、计费影响、隔离细节以及按次运行的 Bamboo token 契约，请参见 [`codex-executor.md`](codex-executor.md)。Lotus 使用 `POST /bamboo/config/validate` 校验本节内容；其二进制 Detect 动作调用 `POST /bamboo/config/codex/detect`，该端点只有在通过了与 worker 派生时相同的前置检查之后，才会返回解析出的 `path` 和 `version`。

## MCP 服务器

磁盘上的键为 `mcpServers`（旧版别名 `mcp` 仍会被读取），类型化字段为 `Config.mcp: McpConfig`：

```json
{
  "mcpServers": {
    "version": 1,
    "servers": [
      {
        "id": "filesystem",
        "name": "Local filesystem",
        "enabled": true,
        "transport": {
          "type": "stdio",
          "command": "npx",
          "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/allow"]
        },
        "request_timeout_ms": 60000,
        "healthcheck_interval_ms": 30000,
        "allowed_tools": [],
        "denied_tools": []
      }
    ]
  }
}
```

`transport` 是三种形态之一（以 `type` 标识）：`stdio`（`command`/`args`/`cwd`/`env`/`startup_timeout_ms`——派生一个子进程）、`sse`（`url`/`headers`/`connect_timeout_ms`）或 `streamable_http`（形态与 `sse` 相同，是 MCP 较新的单端点传输）。`reconnect` 控制自动重连的退避策略（`enabled`、`initial_backoff_ms`、`max_backoff_ms`、`max_attempts`，0 = 不限次数）。`allowed_tools`/`denied_tools` 过滤服务器声明的工具中实际暴露哪些（`allowed_tools` 为空 = 全部允许）。免手工编辑 JSON 的管理方式参见 [`bamboo mcp` CLI 子命令](../README.md#other-subcommands)；通过 SDK 以编程方式接入服务器参见 [`examples/mcp_client.rs`](../examples/mcp_client.rs)。

## 通知

键 `notifications`（`NotificationsConfig`）：

```json
{
  "notifications": {
    "desktop": { "enabled": true },
    "ntfy": { "enabled": true, "base_url": "https://ntfy.sh", "topic": "my-bamboo-alerts", "credential_ref": "notification.ntfy.token", "configured": true },
    "bark": { "enabled": false, "base_url": "https://api.day.app", "credential_ref": "notification.bark.device_key", "configured": false }
  }
}
```

`desktop.enabled: Option<bool>`——`None` 时自动检测（独立运行的 `bamboo serve` 为开启；以 `--parent-pid` 边车方式运行时为关闭，因为此时通常由宿主应用负责通知）。`ntfy`/`bark` 是推送中继渠道；`ntfy.token`/`bark.device_key` 只存在于隔离的加密凭据存储中。普通的 `config.json` 和可解析的轮转备份只包含稳定的 `credential_ref`/`configured` 元数据，绝不包含明文、密文或 UI 掩码。旧版的明文/密文会通过可恢复的配置/凭据清单幂等地完成迁移。

`GET /bamboo/config/notifications` 返回当前的凭据修订号、健康状态、来源、渠道元数据以及各渠道的 configured/source/update 状态，不含任何密钥槽位。通知更新使用 `POST /bamboo/config`，请求中只带 `expected_revision` 和 `notifications`。省略密钥表示保留，`null` 或 `""` 表示清除，非空字符串则替换；掩码以及客户端提供的 `credential_ref`/`configured`/密文都会被拒绝。通知变更不能在同一次调用中与其他根域组合。`"notifications": null` 是显式的域重置：两份凭据都会被清除，通知元数据在同一事务中回到默认值。`bamboo config set notifications.ntfy.token ...` 以及 Bark 的等价命令都经由同一清单事务。

三个渠道都汇入同一个 `AgentEvent::Notification` 类别/优先级策略——参见 `crates/infra/bamboo-notification`。

## `connect`——IM 桥接

通过 IM 平台（Telegram、Feishu/Lark）驱动 session。**尽管 `Config` 有一个类型化的 `connect` 字段，它并不存储在 `config.json` 中**——而是存放在自己的同级文件 `${data_dir}/connect.json` 里，由 `Config::merge_connect_config` 加载/合并、由 `Config::save_connect_config` 保存（两者均在 `config.rs` 中）。一份旁边没有 `connect.json` 且没有旧版内联 `connect` 键的 `config.json` 会启动**零**个后台任务——默认完全惰性。

```json
{
  "platforms": [
    {
      "id": "b3f5...",
      "type": "telegram",
      "token": "123456:ABC-DEF...",
      "allow_from": ["123456789"],
      "admin_from": []
    }
  ]
}
```

字段（`ConnectPlatformConfig`）：`id`（稳定的 UUID，保存时自动回填——千万不要假设手写条目里一定有它）、`type`（`"telegram"` \| `"feishu"`；无法识别的值会被跳过并给出启动警告，而不是硬失败）、`token`/`token_encrypted`（bot token，Telegram）、`app_id`（Feishu，非密钥）、`app_secret`/`app_secret_encrypted`（Feishu）、`domain`（仅 Feishu——`None`/`"feishu"` → `open.feishu.cn`，`"lark"` → `open.larksuite.com`，或自托管部署的显式 `https://` 基地址）、`allow_from`（**为空 = 全部拒绝**——有意比本代码库中其他白名单更严格，因为 IM 桥接天然面向互联网）、`admin_from`（会解析，目前未使用）。

如果 `config.json` 内发现旧版内联 `connect` 键（拆分之前遗留的），下次加载时会自动迁移：收编进 `connect.json`，然后从 `config.json` 中剥离。损坏的 `connect.json` 会被隔离为 `connect.json.bak` 并视为空（故障安全——绝不会静默回退到过期的内联副本）。

## `plugin_trust`

键 `plugin_trust`（`PluginTrustConfig`）——`bamboo plugin install <url>` 的信任策略（参见 [Plugins 使用指南](guides/PLUGINS.md)）：

```json
{
  "plugin_trust": {
    "trusted_hosts": ["github.com/bigduu/"],
    "trusted_keys": [
      { "label": "nova official", "algorithm": "ed25519", "public_key": "<hex>" }
    ],
    "enforcement": "strict"
  }
}
```

`trusted_hosts`——`url` 来源安装的 URL 必须匹配的主机+路径前缀，匹配即可免加 `--allow-untrusted-host`。`trusted_keys`——受信任为 plugin 包签名的 ed25519 公钥（十六进制；默认值内置了官方的 nova 和 magpie 密钥）；经其中之一签名的包可以免加 `--allow-unsigned`，并且按照信任模型也同时满足校验和要求（已验证的签名严格强于粘贴的 `sha256`）。`enforcement`——`"strict"`（默认）或 `"off"`（也接受布尔值：`true`==strict、`false`==off）；`"off"` 相当于在配置层面对每次 `url` 安装都传入 `--insecure`，适用于永远不想要确认弹窗的私有/开发实例。本地（`local_dir`/`local_archive`）安装永远不受该策略约束——它只针对网络下载。

## 关键词脱敏

键 `keyword_masking`（`KeywordMaskingConfig { entries: Vec<KeywordEntry> }`，每个条目为 `{ pattern, match_type: "exact" | "regex", enabled }`）。它以值感知的方式对最终序列化的出站 provider 请求体做整体扫描（而非逐字段）——任何匹配某个模式的字符串值都会在请求离开进程之前被脱敏，从而捕获那些嵌入在工具输出、文件内容等处的密钥，而不只是直接输入到聊天里的那些。

## 权限

存放在 `Config.extra` 内的 `"permissions"` 键下（flatten 兜底区——尚未提升为类型化的顶层字段）。形态（`SerializablePermissionConfig`，`crates/infra/bamboo-permission/src/config.rs`）：`whitelist: Vec<PermissionRule>`、`enabled: bool`、`session_grant_duration_secs`（默认 `1800`）、`mode: Option<PermissionMode>`、`confirm_threshold: Option<RiskLevel>`、`ask_rules: Vec<String>`——形如 `"Bash(rm -rf *)"` 的类 glob 模式，即使在旧版 `bypassPermissions` 模式下也强制弹出确认提示。设计不变式：bypass 会跳过普通提示，但仍会就用户自己的 `ask_rules` 和一小组硬编码的灾难性命令（`sudo`、`curl | sh`、`dd`、`rm -rf /`……）发起询问。更强的 `auto` 模式不发出任何审批提示（包括这些强制询问场景），但仍会执行显式策略和平台级拒绝。

## 模型限额（`model_limits.json`）

一个独立的文件 `${data_dir}/model_limits.json`——用户提供的上下文/输出 token 限额覆盖。显式的用户匹配优先于 provider 运行时元数据。旧版的独立表示是一个裸数组：

```json
[
  { "model_pattern": "my-custom-model", "max_context_tokens": 136192, "max_output_tokens": 8192 }
]
```

`model_pattern` 可以是精确的模型 id，也可以是运行时模型 id 的字面子串；`*` 及其他 glob 字符没有特殊含义。在多个子串匹配中，最长的模式获胜。

`max_context_tokens` 是 provider 的**总输入 + 输出上下文窗口**，而不只是输入额度。Bamboo 按如下方式推导每次请求的输入上限：

```text
max_request_input_tokens =
    max_context_tokens - max_output_tokens - safety_margin
```

`safety_margin` 是可选的。当省略 `max_output_tokens` 时，Bamboo 会从上下文窗口推导它。暴露独立 `max_input_tokens` 和 `max_output_tokens` 的 provider 元数据，会在运行时预算之前被归一到同一个总上下文窗口契约。

模块化配置存储以带版本修订的 `{schema_version, revision, data}` 信封持久化这一节。运行时加载既接受该信封也接受旧版裸数组，并且根 session 会在每个 agent 轮次开始时重新读取该边车文件；显式的 session/子代理级或引擎级 `TokenBudget` 仍然是有意保留的更高优先级覆盖。服务器运行期间，请通过 Bamboo 的设置 API 管理这一节，而不要手工编辑。

在没有匹配的用户值或 provider 值时，Bamboo 回退到全局默认：1M 总上下文 / 每次请求 32K 输出额度。这里有意**不内置按模型的表格**，因此过期的硬编码模型名无法覆盖实时的 provider 元数据。

## 调度任务（`schedules.json`）

这也是一个独立的文件 `${data_dir}/schedules.json`（不属于 `config.json`）——定时/cron 任务，通过 `bamboo schedules list|show|create|delete|run|runs` 或 `/bamboo/schedules` HTTP 路由管理。每个条目（`ScheduleSpec`）包含 `id`/`name`/`enabled`、一个 `trigger`（`Interval` \| `Once` \| `Daily` \| `Weekly` \| `Monthly` \| `Cron`）、可选的 `timezone`、`start_at`/`end_at` 边界、一个 `misfire_policy`（触发时刻进程恰好停机时的处理方式：`RunOnce`（默认）\| `Skip` \| `CatchUpAll` \| `CatchUpWindow`）、一个 `overlap_policy`（`Allow` \| `Skip` \| `QueueOne`，默认 `QueueOne`），以及 `run_config`（被触发运行的 prompt/session 参数）。不适合手工编辑——请使用 CLI/HTTP 子命令，它们会校验触发器的形态。

## 环境变量

Bamboo 读取的所有 `BAMBOO_*` 变量，按影响范围分组。全部可选；文件配置加内置默认值已足以覆盖全新安装。

**引导/核心：**

| 变量 | 作用 |
|---|---|
| `BAMBOO_DATA_DIR` | 数据目录（默认 `${HOME}/.bamboo`）。 |
| `BAMBOO_PORT` | 服务器端口覆盖。 |
| `BAMBOO_BIND` | 服务器绑定地址覆盖。 |
| `BAMBOO_PROVIDER` | 默认 provider 覆盖。 |
| `BAMBOO_HEADLESS` | 启用无头认证模式。 |
| `BAMBOO_WORKERS` | Actix worker 数量覆盖（CLI 层面）。 |

**Provider API key**（仅存于内存——绝不持久化到 `config.json`，即使执行过 `bamboo config set` 也是如此；其意义在于为 Docker/CI/密钥管理器部署提供一份不含明文密钥的配置文件）：

`BAMBOO_OPENAI_API_KEY`, `BAMBOO_ANTHROPIC_API_KEY`, `BAMBOO_GEMINI_API_KEY`.

**记忆开关**（覆盖对应的 `memory.*` 配置字段）：`BAMBOO_MEMORY_PROJECT_PROMPT_INJECTION`、`BAMBOO_MEMORY_RELEVANT_RECALL`、`BAMBOO_MEMORY_RELEVANT_RECALL_RERANK`、`BAMBOO_MEMORY_PROJECT_FIRST_DREAM`。

**服务器加固/网络：**

| 变量 | 作用 |
|---|---|
| `BAMBOO_RATE_LIMIT_PER_SECOND` / `BAMBOO_RATE_LIMIT_BURST` | Governor 限流器调优。 |
| `BAMBOO_RATE_LIMIT_TRUST_XFF` / `BAMBOO_RATE_LIMIT_TRUSTED_HOPS` | 在 N 层反向代理之后信任 `X-Forwarded-For`。 |
| `BAMBOO_CSP` | 完整的 Content-Security-Policy 头覆盖。 |
| `BAMBOO_CSP_CONNECT_SRC` | 仅 CSP 的 `connect-src` 指令。 |
| `BAMBOO_CORS_ALLOW_ORIGINS` | CORS 白名单。 |
| `BAMBOO_ENABLE_DEV_ENDPOINTS` | 控制仅开发用的 HTTP 端点。 |
| `BAMBOO_WS_AUTH_DEADLINE_MS` | WS v2 认证握手超时。 |

**workspace/路径：**

| 变量 | 作用 |
|---|---|
| `BAMBOO_WORKSPACE_DIR` | 项目/workspace 目录覆盖。 |
| `BAMBOO_WORKSPACE_ROOT` | session workspace 的根目录（默认 `{data_dir}/workspaces`）。 |
| `BAMBOO_WORKSPACE_CONFINE` | `1`/`true`/`yes` 强制 workspace 路径保持在 `BAMBOO_WORKSPACE_ROOT` 之下；设置了该变量时自动隐含。 |
| `BAMBOO_SKILL_MODE` | 当前 skill 模式覆盖。 |

**Provider/运行时调优：**

| 变量 | 作用 |
|---|---|
| `BAMBOO_LLM_MAX_RETRIES` / `BAMBOO_LLM_RETRY_BASE_DELAY_MS` / `BAMBOO_LLM_RETRY_MAX_DELAY_MS` | LLM HTTP 请求重试策略。 |
| `BAMBOO_RESPONSES_DEBUG` / `BAMBOO_RESPONSES_DEBUG_FILE` | 将原始 OpenAI Responses API 流量转储到文件以便调试。 |
| `BAMBOO_PYTHON` | Python 解释器覆盖。 |

**Windows 专用：**`BAMBOO_WINDOWS_BASH_PATH`、`BAMBOO_WINDOWS_CMD_TRACE`（同时兼容 `BODHI_WINDOWS_CMD_TRACE`）。

**密钥/plugin/broker：**

| 变量 | 作用 |
|---|---|
| `BAMBOO_CONFIG_ENCRYPTION_KEY` | 静态密钥加密的主 AES-256 密钥——参见[静态加密](#encryption-at-rest)。 |
| `BAMBOO_BROKER_TOKEN` | `bamboo broker`/`broker-agent` 子命令的认证 token。 |
| `BAMBOO_PLUGIN_SERVICE_CONFIG` | 传递给 plugin 服务自身子进程的配置路径。 |
| `BAMBOO_FRONTEND_PACKAGE` | 覆盖内置前端静态包路径。 |

以上所有变量都通过普通的 `std::env::var` 读取，因此也可以通过进程管理器/Docker Compose/systemd unit 设置，而不必在 shell 中 export。

## 密钥与脱敏

每个密钥字段（`providers.*.api_key`、`provider_instances.*.api_key`、`notifications.ntfy.token`、`notifications.bark.device_key`、`connect.platforms[].token`/`.app_secret`、`subagents.broker.token`、`cluster_fabric` 节点 SSH 凭据、密钥型 `env_vars` 条目）在所有读写位置都遵循同一契约：

- **读取（`GET`/`bamboo config`）：**已配置的密钥绝不会以明文回显，而是被替换为字面字符串 `****...****`；如果字段根本未配置，则该键被完全省略（而不是发送 `""`）。
- **写入（`PATCH`/`bamboo config set`）：**一个提交的值当且仅当在去除首尾空白后**完全由 `*` 和/ `.` 字符组成**时——即与掩码占位符的形态完全一致——才被视为**"保持现有密钥不变"**。这是对整个值的检查，不是子串检查：见 `crates/infra/bamboo-config/src/patch.rs` 中的 `is_masked_api_key()`。**空字符串**显式清除密钥。其他任何值——包括因 UI 预填内容未完全清空就粘贴而仍以占位符开头的字符串（例如 `****...****sk-newkey123`）——都会被当作**真正的新密钥**并应用。

  这条整值规则是有意为之：更早的基于子串的检查（已作为 issue #430 修复）可能会在占位符未被完全选中/覆盖时静默丢弃用户粘贴的密钥。任何嵌入 Bamboo 设置 UI 的客户端都绝不能用掩码占位符预填可编辑的密钥字段——留空即表示"保持不变"。

## 静态加密

每个 `*_encrypted` 字段都使用 AES-256-GCM（`crates/infra/bamboo-config/src/encryption.rs`）；磁盘上密文以 `hex(nonce):hex(ciphertext)` 的形式存储，每次加密都使用新的随机 nonce。主密钥按优先级顺序在每个进程内解析一次：

1. **`BAMBOO_CONFIG_ENCRYPTION_KEY`**——十六进制编码，必须恰好解码为 32 字节。优先级最高；适用于由你在外部管理密钥的可复现/临时部署（容器、CI）。
2. **密钥文件** `${data_dir}/.bamboo_encryption_key`——十六进制编码的 32 字节，在 Unix 上以 `0600` 权限原子写入。
3. **机器派生密钥**——对机器标识符（Linux 上的 `/etc/machine-id`、Windows 上的注册表 `MachineGuid`、macOS 上的 `ioreg IOPlatformUUID`）做带域分隔的 SHA-256，然后**持久化到密钥文件**，使后续运行不必重新推导。
4. **最后手段**——密码学随机的 32 字节，持久化到密钥文件。

**备份/灾难恢复提示：**在没有稳定机器标识符的主机上（且未设置 `BAMBOO_CONFIG_ENCRYPTION_KEY`）丢失密钥文件，会使该数据目录中的所有 `*_encrypted` 字段永久无法解密——如果你会备份 Bamboo 数据目录，请把 `.bamboo_encryption_key` 与 `config.json` 一起备份。

## 配置损坏恢复

如果 `${data_dir}/config.json` 存在但无法解析，`Config::from_data_dir` 不会崩溃，也不会静默重置为默认值——而是执行一个恢复流程（issue #493 及其前身），大致如下：

1. **隔离**：将无法解析的原始文件*复制*到 `config.json.corrupted.<timestamp>`（损坏的文件绝不会被删除或移动——原件原封不动地留在 `config.json`）。
2. **恢复**，按顺序尝试以下策略：
   - **抢救（Salvage）**——把损坏文件当作通用 JSON 解析，逐个把顶层键采纳到可用的最佳基线上，只有应用该键后整个 `Config` 仍能反序列化时才保留它。
   - **备份（Backup）**——如果抢救一无所获，则回退到 `config.json.bak`、`.bak.1`、`.bak.2`（按最新优先；保留 3 代，每次成功保存时轮转）。
   - **默认值（Defaults）**——如果都不可行，则使用全新的默认 `Config`。
3. 恢复出的配置会在内存中打上标记（`recovery_status`，绝不持久化），记录它由哪种策略产生、抢救回了哪些字段。
4. **恢复出的配置绝不会被自动保存。**在恢复未确认期间 `Config::save_to_dir` 拒绝写入，因此磁盘上被隔离的损坏原件会一直保留，直到有代码显式调用 `Config::confirm_recovery()` / `confirm_recovery_and_save_to_dir()`（设置 UI/CLI 在向用户展示恢复结果之后才会这样做）。

净效果：损坏的 `config.json` 绝不会造成静默数据丢失——你要么拿回自己的值（抢救/备份），要么在任何内容被覆盖之前得到一个显式、可确认的提示。
