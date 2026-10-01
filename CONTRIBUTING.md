# 为 Bamboo 贡献

首先，感谢你考虑为 Bamboo 贡献！正是像你这样的人让 Bamboo 成为一个出色的工具。

## 行为准则

本项目及所有参与者均受 [Bamboo 行为准则](CODE_OF_CONDUCT.md)约束。参与本项目即表示你应遵守该准则。

## 我可以如何贡献？

### 报告 Bug

在创建 bug 报告之前，请先查看 issue 列表，也许你会发现根本不必新建。创建 bug 报告时，请尽可能包含详细信息：

- **使用清晰、有描述性的标题**
- **描述重现问题的确切步骤**
- **提供具体示例来演示这些步骤**
- **描述你观察到的行为以及你期望的行为**
- **如有帮助，附上日志和截图**
- **说明你的环境**（操作系统、Rust 版本、bamboo 版本）

### 提出增强建议

增强建议以 GitHub issue 的形式跟踪。创建增强建议时，请包含：

- **使用清晰、有描述性的标题**
- **详细描述所建议的增强**
- **解释这个增强为什么有用**
- **列举一些使用方式的示例**
- **说明它影响哪个模块/组件**

### Pull Request

- 填写必需的模板
- PR 标题中不要包含 issue 编号
- 尽可能在 pull request 中附上截图和动画 GIF
- 遵循 Rust 代码风格指南
- 为新功能编写测试
- 为已变更的功能更新文档
- 所有文件以换行符结尾

## 开发环境搭建

### 前置要求

- Rust 1.95 或更高版本
- Cargo
- Git

### 配置你的开发环境

1. Fork 并克隆仓库：
   ```bash
   git clone https://github.com/YOUR_USERNAME/bamboo.git
   cd bamboo
   ```

2. 从集成分支创建你的变更分支：
   ```bash
   git checkout dev
   git pull --ff-only
   git checkout -b feature/my-new-feature
   ```

3. 构建项目：
   ```bash
   cargo build
   ```

4. 运行测试：
   ```bash
   cargo test
   ```

5. 运行服务器：
   ```bash
   cargo run -- serve
   ```

### 运行测试

```bash
# 在最低支持的 Rust 版本上校验完整 workspace
cargo +1.95.0 check --locked --workspace --all-targets --all-features

# 运行全部测试
cargo test

# 运行特定测试套件
cargo test --test server_integration

# 以详细输出运行测试
cargo test -- --nocapture

# 运行特定测试
cargo test test_bamboo_config_default
```

### 代码风格

我们遵循标准 Rust 惯例：

- 使用 `cargo fmt` 格式化代码
- 使用 `cargo clippy` 捕获常见错误
- 为公开 API 编写文档注释
- 遵循 [Rust API 指南](https://rust-lang.github.io/api-guidelines/)

### 提交信息

- 使用一般现在时（"Add feature" 而非 "Added feature"）
- 使用祈使语气（"Move cursor to..." 而非 "Moves cursor to..."）
- 首行不超过 72 个字符
- 首行之后可随意引用 issue 和 pull request
- 考虑以适用的 emoji 开头：
  - 🎨 `:art:` 改进代码格式/结构时
  - 🐎 `:racehorse:` 提升性能时
  - 🚱 `:non-potable_water:` 堵住内存泄漏时
  - 📝 `:memo:` 撰写文档时
  - 🐛 `:bug:` 修复 bug 时
  - 🔥 `:fire:` 删除代码或文件时
  - 💚 `:green_heart:` 修复 CI 构建时
  - ✅ `:white_check_mark:` 添加测试时
  - 🔒 `:lock:` 处理安全问题时
  - ⬆️ `:arrow_up:` 升级依赖时
  - ⬇️ `:arrow_down:` 降级依赖时

### 项目结构

Bamboo 使用 Cargo workspace，`crates/` 下包含以下 crate：

```
bamboo/
├── src/                    # 主 crate（bamboo-agent 根）
│   └── bin/bamboo.rs       # CLI 二进制入口
├── crates/
│   ├── bamboo-agent-core/  # Agent 运行时核心、组合、存储、工具
│   ├── bamboo-compression/ # 上下文压缩与摘要
│   ├── bamboo-domain/      # 领域类型：会话、工具、工作流、调度、MCP
│   ├── bamboo-engine/      # Agent 引擎：MCP、指标、运行时、skill
│   ├── bamboo-infrastructure/ # 配置、LLM provider、进程管理、存储
│   ├── bamboo-memory/      # 记忆系统：持久记忆、预算、Dream 笔记本
│   ├── bamboo-server/      # HTTP 服务器、handler、路由、应用状态
│   └── bamboo-tools/       # 工具注册表、执行器、编排器、内置工具
├── tests/                  # 集成测试
├── Cargo.toml              # workspace 清单
└── README.md
```

### Workspace crate 职责

| Crate | 职责 |
|---|---|
| `bamboo-agent-core` | Agent 系统组合、workspace 状态、核心 Agent 类型 |
| `bamboo-compression` | 上下文压缩、摘要、token 限制 |
| `bamboo-domain` | 会话、工具、工作流、调度、MCP 配置的领域类型 |
| `bamboo-engine` | Agent 引擎：MCP 集成、指标、运行时、skill 执行 |
| `bamboo-infrastructure` | 配置管理、LLM provider、进程管理、SQLite 存储 |
| `bamboo-memory` | 记忆系统、token 预算管理、Dream 笔记本 |
| `bamboo-server` | HTTP 服务器、请求 handler、路由、会话应用状态 |
| `bamboo-tools` | 工具注册表、执行器、编排器、内置工具、权限系统 |

### 模块准则

- 每个模块应有清晰的职责
- 使用 `mod.rs` 再导出公开 API
- 为所有公开条目编写文档
- 在模块内包含单元测试
- 保持模块依赖最少

### 测试准则

- 为所有新功能编写测试
- 提交 PR 前确保所有测试通过
- 使用描述性的测试名
- 测试中包含边界情况
- 异步测试使用 `#[tokio::test]`
- 需要文件系统访问的测试使用 `tempfile`
- 在 macOS 上，使用 `scripts/run-macos-server-lib-tests.sh` 运行单体
  `bamboo-server` lib 测试。该库包直接链接的测试/示例产物会禁用
  Apple 的紧凑展开表，因为该 crate 的 DWARF 展开记录超出了该格式
  16 MiB 的偏移限制。DWARF 展开与行号表仍为 panic 回溯和调试启用。
  该标志在生成 rlib 时没有最终链接效应，也不会传播到下游
  dev/release 二进制。

### 文档准则

- 变更功能时更新 README.md
- 用 `///` 注释更新 API 文档
- 文档中包含示例
- 保持 CHANGELOG.md 更新
- 为复杂逻辑添加行内注释

## 发布流程

1. 用新版本更新 CHANGELOG.md
2. 更新 Cargo.toml 中的版本号
3. 创建 git tag：`git tag v0.x.0`
4. 推送 tag：`git push origin v0.x.0`
5. CI 会自动发布到 crates.io

## 补充说明

### Issue 与 Pull Request 标签

- `bug` - 某些功能不工作
- `enhancement` - 新功能或请求
- `documentation` - 文档改进或补充
- `good first issue` - 适合新手
- `help wanted` - 需要额外关注
- `wontfix` - 不会处理

## CI/CD 设置

### 现有工作流

Bamboo 使用 GitHub Actions 进行持续集成与发布：

- **CI**（`.github/workflows/ci.yml`）—— 进入 `dev` 的 pull request 会在必需的 `Test` 门中运行锁定版本的 Rust 构建/测试、格式化以及 CI 工作流策略检查。进入 `main` 的 pull request、推送到 `main` 以及手动派发保留全面验证，全特性库与集成套件位于必需的 `E2E Tests` 作业中。只有本仓库 `dev` 分支进入 `main` 的晋升 pull request 才会附加 Linux、macOS 和 Windows 上的发布构建；手动派发也会运行该平台矩阵。Linux TLS 与前端契约测试在 `Test` 中运行，而 macOS 和 Windows 运行各自平台特定的检查。成功的 dev PR 构建可以复用自己的 Rust 缓存直至关闭；随后 `.github/workflows/pr-cache-cleanup.yml` 只移除同一仓库该 PR merge-ref 的缓存。
- **CodeQL**（`.github/workflows/codeql.yml`）—— 对进入 `main` 的 pull request、推送到 `main` 以及显式手动派发运行 Actions、JavaScript/TypeScript、Python 和 Rust 分析。日常 `dev` 活动不运行 CodeQL。
- **Publish Crate**（`.github/workflows/publish-crate.yml`）—— 按依赖顺序把 workspace crate 发布到 crates.io。通常由 Zenith 发布列车派发，携带统一的日期版本和要嵌入的 `@bigduu/lotus` 前端版本；支持 `dry_run`。
- **Publish Docker image**（`.github/workflows/docker-publish.yml`）—— 构建多架构容器镜像并推送到 GHCR。
- **Documentation**（`.github/workflows/docs.yml`）—— 每次推送到 main 时构建文档，部署到 GitHub Pages。

### 徽章 URL

工作流运行后，徽章解析到：

- CI：`https://github.com/bigduu/Bamboo-agent/actions/workflows/ci.yml`
- 文档：`https://github.com/bigduu/Bamboo-agent/actions/workflows/docs.yml`
- GitHub Pages：`https://bigduu.github.io/Bamboo-agent/`
- docs.rs：`https://docs.rs/bamboo-agent`（发布到 crates.io 后自动构建）

### 设置清单

1. 推送变更到 GitHub 以触发 CI。
2. 启用 GitHub Pages：**Settings > Pages > Source** 设为 **GitHub Actions**。
3. 在 **Settings > Secrets and variables > Actions** 下添加 `CARGO_REGISTRY_TOKEN` secret，用于 crates.io 发布。
4. 推送后在 README 中确认徽章状态。

## E2E 测试

```bash
# 运行全部 e2e 测试
cargo test --test e2e

# 运行特定测试
cargo test --test e2e test_health_endpoint
```

测试覆盖所有 API 端点（chat、execute、events、sessions、tasks、respond、metrics、MCP、health）。每个测试都使用 actix-web 的内存测试框架实现隔离。

### 添加 E2E 测试

1. 创建 `tests/e2e/new_endpoint.rs`
2. 使用 `common` 中的 `create_test_app()` 辅助函数
3. 把模块加入 `tests/e2e/mod.rs`

## 有疑问？

欢迎打开带 question 标签的 issue，或在 GitHub 上发起讨论。

---

感谢你的贡献！
