# 更新日志

本文件记录本项目的所有重要变更。

格式基于 [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)。
历史上的 `0.x` 版本遵循[语义化版本](https://semver.org/spec/v2.0.0.html)；
此后项目发布**按日期编号的 nightly 版本**（见下文）。

## [Unreleased] — nightly（按日期编号）

自 `0.3.0` 起，项目发布由发布列车裁剪的**按日期编号的 nightly 版本**（如 `2026.7.x`），
而非 SemVer 点版本。nightly 之间的变更记录在 git 历史与已合并 PR（事实源）中；
下面的 SemVer 小节仅为历史上的 `0.x` 版本保留。

### Changed

- 将最低支持的 Rust 版本从 1.84 提升到 1.95，以匹配锁定的依赖图，
  并新增显式 MSRV CI 车道与依赖图审计。

## [0.3.0] - 2026-02-26

### Security
- provider API 密钥现在在 `config.json` 中静态加密，且绝不通过 API 返回（仅掩码占位符）。
- MCP SSE 头值和 MCP stdio 环境变量值现在在 `config.json` 中静态加密，且绝不通过 API 返回（仅掩码占位符）。

## [0.2.12] - 2026-02-26

### Changed
- 配置现在完全统一到 `config.json`（单一入口/出口）：关键词掩码、模型映射、MCP 服务器配置和权限都通过统一配置持久化。

### Fixed
- MCP 服务器管理端点现在会把变更持久化到统一配置（运行时与磁盘之间不再漂移）。

## [0.2.11] - 2026-02-26

### Fixed
- Copilot 认证端点现在遵循已配置的 HTTP/HTTPS 代理设置（支持企业网络）。
- 内置 `http_request` 工具现在遵循相同的已配置代理设置（出站网络行为一致）。

## [0.2.10] - 2026-02-26

### Added
- 工具执行流式输出：工具现在可以在运行期间发出增量输出事件（SSE `tool_token`）。
- Claude Code CLI 集成：可选内置 `claude_code` 工具（发现 `claude` 可执行时自动启用），以 `--output-format stream-json` 运行并流式输出。

### Changed
- 流式输出：工具发出的 `token` 事件被当作工具范围的输出（`tool_token`），不再混入助手文本流。
- 模型限制：为 `gpt-4.1` 新增内置上下文窗口条目（默认 128k；如有需要可通过用户配置覆盖）。

### Fixed
- Claude Code CLI：使用 `-p/--print` 加 `--output-format=stream-json` 时始终传 `--verbose`（Claude Code 的要求）。

## [0.2.9] - 2026-02-25

### Added
- Copilot：支持通过 `providers.copilot.model` 选择并持久化 provider 特定的默认模型。
- Copilot：OpenAI 兼容的 `/v1/chat/completions` 现在会把解析后的模型转发到 Copilot 上游请求。

### Changed
- 设置：澄清 `POST /bamboo/settings/provider` 本身就会保存并重载 provider 配置（因此通常无需再单独调用重载）。

## [0.2.8] - 2026-02-25

### Removed (Breaking)
- 移除遗留的 `bamboo_agent::agent::server::state::AppState` 类型/模块；Bamboo 现在只有一个统一的 `AppState`。
  - **迁移**：一律改用 `bamboo_agent::server::app_state::AppState`（或 `bamboo_agent::server::AppState`）。
- 移除遗留的服务器实现/模块：
  - `bamboo_agent::agent::server`（遗留 Actix 服务器）
  - `bamboo_agent::web_service`（代理服务器）
- 移除遗留的 `/api/v1/stream/{session_id}` 端点。请改用 `POST /api/v1/execute/{session_id}` + `GET /api/v1/events/{session_id}`。

### Fixed
- 修复了当 handler 需要错误的 `AppState` 类型时可能导致运行时故障（如 `/anthropic/v1/messages`）的 Actix `Data<T>` 提取器不匹配问题。

## [0.2.6] - 2025-02-25

### Fixed - 关键生产阻塞问题

#### Security
- **安全**：修复 `bamboo config` 命令的 API 密钥泄露——密钥现在默认打码
  - 新增 `--show-secrets` 标志，在需要时显式显示 API 密钥
  - 防止 API 密钥出现在 shell 历史、CI 日志或屏幕共享中

#### 配置系统
- **严重**：修复运行中的服务器不遵守 `--data-dir` 标志的问题
  - 服务器现在会从指定的数据目录正确加载配置
  - `AppState::new()` 和 `reload_config()` 使用 `Config::from_data_dir()`
  - 修复潜在的数据损坏与安全问题

#### 文档
- **高**：修复配置模块的文档漂移
  - 配置文件位置更新为 `${BAMBOO_DATA_DIR}/config.json`（默认 `${HOME}/.bamboo/config.json`；此前错误地显示为 XDG 路径）
  - 移除 TOML 格式引用（实际格式只有 JSON）
  - 修正环境变量名：`BAMBOO_HEADLESS`（此前错误地写为 `BAMBOO_HEADLESS_AUTH`）
  - 移除对 `HTTP_PROXY`/`HTTPS_PROXY` 的提及（实现显式忽略它们）
  - 记录正确的优先级顺序：CLI > 环境变量 > 文件 > 默认值
  - 所有 provider 配置示例从 TOML 转为 JSON

### Changed

#### 默认 Provider
- **变更**：默认 provider 从 "copilot" 回退为 "anthropic"
  - **原因**：Copilot OAuth2 认证在 CI/CD 环境中难以测试与模拟
  - **影响**：新安装将默认使用 Anthropic（需要 API 密钥）
  - **迁移**：想用 Copilot 的用户应在配置中显式设置 `provider: "copilot"`
  - **未来**：待测试基础设施就绪后，Copilot 将在 v0.4.0 重新成为默认

- 配置文档现在准确反映实现行为
- 所有 provider 配置示例统一使用 JSON 格式

### 架构
- 统一配置系统，单一 `Config` 结构体
- 正确的优先级顺序：CLI 参数 > 环境变量 > 配置文件 > 代码默认值
- 服务器配置（端口、绑定、worker 数、static_dir）并入统一 Config

### 已知问题（优先级较低）
- `--workers` CLI 标志已解析但未接入服务器（使用默认 worker 数）
- `--static-dir` CLI 标志已解析但未接入服务器
- 这些问题已记录在案，可在未来版本处理

### 修改的文件
- `src/core/config.rs` - 默认 provider 回退为 "anthropic"
- `src/server/app_state.rs` - 修复配置加载中的 data_dir 使用
- `src/server/handlers/agent_api.rs` - 修复 `get_claude_dir()`，目录缺失时创建
- `src/bin/bamboo.rs` - 新增 `--show-secrets` 标志与密钥打码
- `tests/e2e/copilot_auth.rs` - 针对新默认 provider 更新测试
- `Cargo.toml` - 版本号升至 0.2.6

### 迁移说明
- **默认 provider 变更**：如果你依赖隐式的 "copilot" 默认值，请在配置中显式设置：
  ```json
  {
    "provider": "copilot"
  }
  ```
- **无其他破坏性变更** - 对既有配置 100% 向后兼容
- 用户可选择在 config.json 中添加 `server` 段（省略时使用默认值）
- 环境变量 `BAMBOO_HEADLESS_AUTH` 已弃用，请改用 `BAMBOO_HEADLESS`

### 部署状态
✅ **可用于生产部署**

Codex 评审第 3 轮指出的所有关键生产阻塞问题均已修复。

## [0.2.0] - 2026-02-24

### 🎉 重大重构：统一服务器架构

本版本将 `web_service` 和 `agent::server` 合并为统一的 `server/` 模块，
并以显式路由统一了所有 HTTP handler。

### Added

- **统一服务器模块**（`src/server/`）
  - 单一 `AppState`，直接访问 provider（消除代理模式）
  - 统一的指标基础设施
  - README.md 与 MIGRATION.md 中的完整迁移指南

- **显式路由系统**
  - 所有路由现在使用显式 `web::route()` 注册
  - handler 中不再有 `#[get]`、`#[post]` 宏
  - `src/server/routes.rs` 中的单一事实源（约 120 条路由）

- **统一 handler 术语**
  - 所有 HTTP handler 归并到 `src/server/handlers/` 之下
  - Agent handler：`handlers/agent/`（chat、execute、events 等）
  - Provider handler：`handlers/*.rs`（openai、anthropic、gemini 等）

- **服务器模式**
  - `run()` - 桌面模式（仅 localhost，不限流）
  - `run_with_bind()` - Docker 模式（自定义绑定，限流）
  - `run_with_bind_and_static()` - 带前端服务的生产模式

- **模块组织**
  - 新增 `server::routes` 模块，承载路由配置
  - 新增 `server::server` 模块，承载入口点
  - 新增 `server::config` 模块，承载 CORS/安全头
  - 新增 `server::metrics` 模块，承载统一基础设施

### Changed

- **BREAKING**（保留向后兼容）：
  - 弃用 `agent::server` 模块 → 改用 `server`
  - 弃用 `web_service` 模块 → 改用 `server`
  - 旧导入仍可使用，但会收到弃用警告

- **Handler 结构**：
  - `src/server/handlers/*.rs` → `src/server/handlers/agent/*.rs`（核心 handler）
  - `src/server/controllers/*.rs` → `src/server/handlers/*.rs`（provider handler）
  - 所有 handler 使用一致的术语统一

- **路由注册**：
  - 从：基于宏（`#[get("/path")]`）
  - 到：显式（`.route("/path", web::get().to(handler))`）
  - 全部约 120 条路由现在显式注册

- **状态管理**：
  - 单一统一 `AppState` 取代双重状态
  - 直接访问 provider 取代对自身的 HTTP 回调

- **代码组织**：
  - 消除 24 条重复路由注册（54 → 30，减少 44%）
  - 移除代理模式（`build_agent_state()` 函数）
  - 更清晰的模块结构

### Removed

- 重复路由定义（消除 24 条路由）
- 对自身进行 HTTP 回调的代理模式
- 路由宏，改为显式注册
- 约 430 行冗余代码

### Fixed

- `app_state::tests` 中的异步测试阻塞问题
- 配置保存/加载测试改用临时路径，以兼容 CI
- HTTP 请求测试通过 `BAMBOO_TEST_NETWORK=1` 环境变量选择性启用

### 迁移指南

#### 对库用户

旧写法（已弃用但仍可用）：
```rust
// 注意：该遗留导入路径已在 v0.2.8 中移除。
// use bamboo_agent::agent::server::state::AppState;
use bamboo_agent::web_service::WebService;
use bamboo_agent::agent::server::handlers;
```

新写法（推荐）：
```rust
use bamboo_agent::server::AppState;
use bamboo_agent::server::WebService;
use bamboo_agent::server::handlers;
```

#### 对贡献者

- 所有 HTTP handler 都在 `src/server/handlers/`
- 在 `src/server/routes.rs` 中使用显式路由注册
- 不再使用 `#[get]`、`#[post]` 等宏

### 统计

- **变更文件数**：63
- **新增行数**：+599
- **删除行数**：-1029
- **净变化**：-430 行（更干净的代码库！）
- **测试**：867/867 通过（100%）
- **消除的重复路由**：24（减少 44%）
- **提交**：2-3 天内 8 个主要提交
- 消除代理模式（`build_agent_state`）
- 统一状态管理（单一 AppState）
- 全部 866 个测试更新并通过

### Fixed
- `app_state::tests::test_app_state_creation` 中的异步测试阻塞问题

### 迁移指南

#### v0.2.0 之前
```rust
use bamboo_agent::agent::server::AppState;
use bamboo_agent::web_service::WebService;
```

#### v0.2.0 之后
```rust
use bamboo_agent::server::AppState;
use bamboo_agent::server::WebService;
```

所有旧导入仍可使用，但会收到弃用警告。详情见 MIGRATION.md。

## [0.1.2] - 2026-02-23

### Added
- 首次发布到 crates.io
- 多 LLM provider 支持（OpenAI、Anthropic、Gemini、Copilot）
- 基于 Actix-web 的内置 HTTP 服务器
- 带工具执行的 Agent 循环
- 会话管理
- 工作流系统
- MCP（Model Context Protocol）集成
- 863 个测试通过

[0.2.0]: https://github.com/bigduu/Bamboo-agent/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/bigduu/Bamboo-agent/releases/tag/v0.1.2
