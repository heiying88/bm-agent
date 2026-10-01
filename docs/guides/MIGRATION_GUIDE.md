# 迁移指南

本指南帮助你从旧的 `agent-*` crate 迁移到统一的 `bamboo-agent` crate。

## 概述

Bamboo 现在组织为一个 Cargo workspace，`crates/` 下包含以下 crate：

- `bamboo-agent-core` —— agent 运行时核心、组合、存储、工具
- `bamboo-compression` —— 上下文压缩与摘要
- `bamboo-domain` —— 领域类型：会话、工具、工作流、调度、MCP
- `bamboo-engine` —— agent 引擎：MCP、指标、运行时、skill
- `bamboo-infrastructure` —— 配置、LLM provider、进程管理、存储
- `bamboo-memory` —— 记忆系统：持久记忆、预算、Dream 笔记本
- `bamboo-server` —— HTTP 服务器、handler、路由、应用状态
- `bamboo-tools` —— 工具注册表、执行器、编排器、内置工具

它们取代的是早先那个单体 `bamboo-agent` crate——其内部模块包括 `chat_core`、`agent-core`、`agent-llm`、`agent-tools`、`agent-metrics`、`agent-mcp`、`agent-loop`、`agent-server`、`agent-skill`、`agent-cli` 和 `web_service`。

## 迁移步骤

### 1. 更新 Cargo.toml

**迁移前：**
```toml
[dependencies]
chat_core = { path = "../chat_core" }
agent-core = { path = "../agent-core" }
agent-llm = { path = "../agent-llm" }
agent-tools = { path = "../agent-tools" }
web_service = { path = "../web_service" }
```

**迁移后：**
```toml
[dependencies]
bamboo-agent = "2026.4"
# 或者使用各个 workspace crate：
# bamboo-domain = { path = "../crates/bamboo-domain" }
# bamboo-server = { path = "../crates/bamboo-server" }
# bamboo-tools = { path = "../crates/bamboo-tools" }
```

### 2. 更新导入

#### 核心类型

**迁移前：**
```rust
use chat_core::Config;
use chat_core::paths::bamboo_dir;
use chat_core::keyword_masking::KeywordMaskingConfig;
```

**迁移后：**
```rust
use bamboo_infrastructure::config::Config;
use bamboo_domain::paths::bamboo_dir;
use bamboo_domain::keyword_masking::KeywordMaskingConfig;
```

#### Agent 类型

**迁移前：**
```rust
use agent_core::{AgentError, Session, Message};
use agent_core::tools::{ToolCall, ToolResult, ToolExecutor};
```

**迁移后：**
```rust
use bamboo_agent_core::agent::{AgentError, Session, Message};
use bamboo_tools::{ToolCall, ToolResult, ToolExecutor};
```

#### LLM provider

**迁移前：**
```rust
use agent_llm::{LLMProvider, LLMError};
use agent_llm::providers::{OpenAIProvider, AnthropicProvider};
use agent_llm::create_provider;
```

**迁移后：**
```rust
use bamboo_infrastructure::llm::{LLMProvider, LLMError};
use bamboo_infrastructure::llm::providers::{OpenAIProvider, AnthropicProvider};
use bamboo_infrastructure::llm::create_provider;
```

#### 工具

**迁移前：**
```rust
use agent_tools::{BuiltinToolExecutor, ToolRegistry};
use agent_tools::tools::ReadFileTool;
```

**迁移后：**
```rust
use bamboo_tools::{BuiltinToolExecutor, ToolRegistry};
use bamboo_tools::tools::ReadFileTool;
```

#### 指标

**迁移前：**
```rust
use agent_metrics::{MetricsBus, MetricsWorker};
```

**迁移后：**
```rust
use bamboo_engine::metrics::{MetricsBus, MetricsWorker};
```

#### Web 服务

**迁移前（v0.1.x）：**
```rust
use bamboo::web_service::WebService;
use bamboo::web_service::controllers::agent_controller;
```

**迁移后（v0.1.x）：**
```rust
use bamboo::web_service::WebService;
use bamboo::web_service::controllers::agent_controller;
```

**最新（v0.2.0+ / workspace）：**
```rust
use bamboo_server::WebService;
use bamboo_server::handlers;
// handler 位于 crates/bamboo-server/src/handlers/ 下
```

#### Claude 集成

**迁移前：**
```rust
// 位于 src-tauri 中
use crate::claude::find_claude_binary;
use crate::command::slash_commands::SlashCommand;
use crate::command::workflows::save_workflow;
```

**迁移后：**
```rust
use bamboo_server::claude_runner::find_claude_binary;
use bamboo_tools::slash_commands::SlashCommand;
use bamboo_server::workflow::save_workflow;
```

### 3. 更新函数调用

大多数函数调用保持不变，但部分路径有变化：

#### 创建 provider

**迁移前：**
```rust
let provider = agent_llm::create_provider(&config)?;
```

**迁移后：**
```rust
let provider = bamboo_infrastructure::llm::create_provider(&config)?;
```

#### 工具执行

**迁移前：**
```rust
let executor = agent_tools::BuiltinToolExecutor::new();
let result = executor.execute(&tool_call).await;
```

**迁移后：**
```rust
let executor = bamboo_tools::BuiltinToolExecutor::new();
let result = executor.execute(&tool_call).await;
```

### 4. 更新配置

Bamboo 现在为所有配置和数据使用统一的数据目录：
- `BAMBOO_DATA_DIR`（默认 `${HOME}/.bamboo`）

**迁移前：**
```rust
let config_dir = dirs::home_dir().unwrap().join(".bamboo");
```

**迁移后：**
```rust
let config_dir = bamboo_infrastructure::config::paths::bamboo_home();
let data_dir = bamboo_infrastructure::config::paths::bamboo_home();
```

也可以使用提供的辅助函数：

```rust
use bamboo_infrastructure::config::paths;

let config_path = paths::config_json_path();
let sessions_dir = paths::sessions_dir();
let workflows_dir = paths::workflows_dir();
```

### 5. 更新服务器配置

**迁移前：**
```rust
use web_service::WebService;

let server = WebService::new(
    data_dir,
    provider,
    config,
    metrics_bus,
);
```

**迁移后：**
```rust
use bamboo_server::WebService;

let server = WebService::new(
    data_dir,
    provider,
    config,
    metrics_bus,
);
```

**最新（workspace）：**
```rust
use bamboo_server::app_state::AppState;
use bamboo_server::routes;

let app = bamboo_server::build_app(data_dir, config);
```

## API 变更

### ToolSchema 结构

**迁移前：**
```rust
let schema = ToolSchema {
    name: "read_file".to_string(),
    description: "...".to_string(),
    parameters: json!({}),
};
```

**迁移后：**
```rust
let schema = ToolSchema {
    schema_type: "function".to_string(),
    function: FunctionSchema {
        name: "read_file".to_string(),
        description: "...".to_string(),
        parameters: json!({}),
    },
};
```

### KeywordEntry 字段

**迁移前：**
```rust
let entry = KeywordEntry {
    pattern: "secret".to_string(),
    mask_type: MaskType::Exact,
    replacement: "***".to_string(),
    case_sensitive: false,
};
```

**迁移后：**
```rust
let entry = KeywordEntry {
    pattern: "secret".to_string(),
    match_type: MatchType::Exact,
    enabled: true,
};
```

## 常见迁移模式

### 模式 1：使用 prelude

创建一个 prelude 模块来简化导入：

```rust
// src/prelude.rs
pub use bamboo_domain::session::{Session, Message, Role};
pub use bamboo_infrastructure::llm::LLMProvider;
pub use bamboo_tools::BuiltinToolExecutor;
pub use bamboo_infrastructure::config::Config;

// 在你的代码中
mod prelude;
use prelude::*;
```

### 模式 2：类型别名

如果类型引用很多，可以创建别名：

```rust
type Provider = bamboo_infrastructure::llm::LLMProvider;
type Executor = bamboo_tools::BuiltinToolExecutor;
type Result<T> = std::result::Result<T, bamboo_agent_core::AgentError>;
```

### 模式 3：重导出常用类型

在你的 lib.rs 中：

```rust
pub use bamboo_agent_core::{
    Session, Message, AgentError,
};
pub use bamboo_infrastructure::config::Config;
```

## 测试你的迁移

1. **运行 cargo check**：`cargo check`
2. **运行测试**：`cargo test`
3. **检查导入**：查找是否还残留旧 crate 的引用
4. **测试功能**：确保所有功能按预期工作

## 故障排查

### 错误：“cannot find type `Session` in crate `bamboo`”

**解决方案**：更新导入路径。`Session` 现在位于 `bamboo_domain::session::Session`。

### 错误：“no field `name` on type `ToolSchema`”

**解决方案**：`ToolSchema` 现在是嵌套结构，通过 `schema.function.name` 访问名称。

### 错误：“unresolved import `chat_core`”

**解决方案**：把所有 `chat_core` 导入替换为 `bamboo_domain`（领域类型）或 `bamboo_infrastructure`（配置、llm）。

### 错误：“no module named `agent_loop`”

**解决方案**：loop 模块现在是 `bamboo_engine::runtime`（engine crate 负责 agent 运行时和执行）。

## 更多资源

- [API 文档](https://docs.rs/bamboo-agent)
- [仓库](https://github.com/bigduu/Bamboo-agent)
- [GitHub Issues](https://github.com/bigduu/Bamboo-agent/issues)

## 获取帮助

如果迁移过程中遇到问题：

1. 查阅 [API 文档](https://docs.rs/bamboo-agent)
2. 搜索[已有 issue](https://github.com/bigduu/Bamboo-agent/issues)
3. 提一个带“migration”标签的新 issue
4. 发起一个[讨论](https://github.com/bigduu/Bamboo-agent/discussions)

## 更新日志

完整变更列表见 [CHANGELOG.md](../../CHANGELOG.md)。

## v0.2.0 服务器整合

v0.2.0 引入了一次重大重构，把双服务器架构整合为统一模块。

### 关键变更

1. **workspace crate**：单体 `bamboo-agent` crate 拆分为 `crates/bamboo-server`、`crates/bamboo-domain` 等。
2. **显式路由**：所有路由都注册在 `crates/bamboo-server/src/routes/`。
3. **统一 handler**：controller 与 handler 合并进 `bamboo_server::handlers`。
4. **直接访问 provider**：去掉了带 HTTP 回调的代理模式。

### 从 v0.1.x 迁移到 v0.2.0

#### 服务器导入

**迁移前（v0.1.x）：**
```rust
// 注意：这条旧导入路径已在 v0.2.8 中移除。
// use bamboo::agent::server::state::AppState;
use bamboo::agent::server::handlers;
use bamboo::web_service::WebService;
use bamboo::web_service::controllers::*;
```

**迁移后（v0.2.0+ / workspace）：**
```rust
use bamboo_server::app_state::AppState;
use bamboo_server::handlers;
use bamboo_server::WebService;
// 注意：controllers::* → handlers::*
```

#### handler 组织

**agent handler**（位于 `crates/bamboo-server/src/handlers/agent/`）：
- `chat`, `execute`, `events`, `stream`, `stop`, `history`, `respond`, `delete`, `health`, `metrics`, `todo`, `mcp`

**provider handler**（位于 `crates/bamboo-server/src/handlers/`）：
- `openai/`, `anthropic/`, `gemini/`, `copilot_auth/`, `agent_api.rs`, `command/`, `settings/`, `skill/`, `tools/`, `workspace/`

### 向后兼容性

旧导入路径在 v0.2.0 中弃用，并在 v0.2.8 中移除。

```rust
// 旧写法（v0.2.8 中已移除）
// use bamboo::agent::server::state::AppState;
// use bamboo::web_service::WebService;
// use bamboo::server::controllers::agent_api;

// 当前写法（workspace crate）
use bamboo_server::app_state::AppState;
use bamboo_server::WebService;
use bamboo_server::handlers::agent_api;
```

### 收益

- ✅ **无路由重复**：所有路由的单一事实来源
- ✅ **架构更清晰**：workspace crate 按清晰边界分离
- ✅ **性能更好**：直接访问 provider（没有 HTTP 回调）
- **更容易维护**：所有路由都集中在 `crates/bamboo-server/src/routes/` 里一目了然
- ✅ **减少 430 行代码**：更干净、更易维护的代码库

### 更多细节

完整变更历史见 [CHANGELOG.md](../../CHANGELOG.md)。

---

需要帮助？在 GitHub 上提 issue 或发起讨论！
