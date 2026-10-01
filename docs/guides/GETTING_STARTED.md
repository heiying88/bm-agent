# 快速上手

首次运行全流程：安装、配置 provider、用三种方式（CLI、HTTP、进程内 SDK）各跑一轮 agent 交互，最后告诉你接下来去哪里。

## 1. 安装

```bash
cargo install --path .        # 从源码检出目录安装，或者：cargo install bamboo-agent
```

也可以不安装，直接在 workspace 里构建/运行：
`cargo run --bin bamboo -- <subcommand>`。

## 2. 配置 provider

```bash
bamboo init
```

交互式：选择一个 provider（`anthropic`/`openai`/`gemini`/`copilot`/
`bodhi`）并提示输入 API key。面向脚本/CI 的非交互式形式：

```bash
bamboo init --non-interactive --provider anthropic --api-key "sk-ant-..."
```

该命令会写入 `~/.bamboo/config.json`（可用 `--data-dir` 覆盖），并把密钥**静态加密**存储（参见[静态加密](../config-reference.md#encryption-at-rest)）。随时可以用下面的命令验证安装是否健康：

```bash
bamboo doctor    # 配置存在、provider 已配好密钥、服务器可达——存在阻断性问题时以非零码退出
```

## 3. 你的第一轮 agent 交互——三种方式

### a) 无界面单次运行（最快看到它跑起来的方式）

```bash
bamboo -p "List the files here and tell me what this project does."
```

启动完整运行时（包括子代理支持），运行一轮，打印结果后退出。之后的调用加上 `-s <session-id>` 即可继续同一会话。

### b) HTTP 服务器 + curl

```bash
bamboo serve &

SID=$(curl -s http://127.0.0.1:9562/api/v1/chat \
  -H 'Content-Type: application/json' \
  -d '{"message":"List the files here and tell me what this project does.","model":"claude-sonnet-4-6"}' \
  | jq -r .session_id)

curl -s -X POST "http://127.0.0.1:9562/api/v1/execute/$SID" \
  -H 'Content-Type: application/json' -d '{}'

curl -N "http://127.0.0.1:9562/api/v1/events/$SID"   # 实时观看运行过程（SSE）
```

`chat` 只负责**持久化**这一轮；真正**运行**循环的是 `execute`；`events` 负责流式输出。完整的 HTTP/SSE 接口请见 [`docs/guides/API.md`](../guides/API.md)。

### c) 进程内 Rust SDK（无服务器）

```rust
use bamboo_sdk::agent::{Agent, Session};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let home = dirs::home_dir().unwrap().join(".bamboo");
    let agent = Agent::builder()
        .model("claude-sonnet-4-6")
        .instruction("You are a helpful coding agent.")
        .with_defaults_for_data_dir(home)
        .await?
        .build()?;

    let session = Session::new("demo-session", "claude-sonnet-4-6");
    let mut rx = agent.run_stream(session, "List the files here and tell me what this project does.");
    while let Some(event) = rx.recv().await {
        println!("{event:?}");
    }
    Ok(())
}
```

它运行与 `bamboo serve` 完全相同的 agent 循环，只是跑在你自己的进程里——[`examples/`](../../examples/) 提供了本例以及其他若干模式（流式消费真实事件类型而非 `Debug` 打印、自定义工具、恢复会话、`ExecuteRequest` 逃生通道、连接 MCP 服务器）的可编译、可运行版本。

## 4. 接下来看什么

| 想要…… | 阅读 |
|---|---|
| 了解每个 `config.json` 键 | [配置参考](../config-reference.md) |
| 从 Telegram/飞书驱动 Bamboo | [Connect / IM 桥接指南](./CONNECT.md) |
| 安装/信任一个插件（如 Nova） | [插件指南](./PLUGINS.md) |
| 以长期运行的服务器方式部署 | [部署指南](./DEPLOY.md) |
| 把 agent 循环嵌入你自己的 Rust 应用 | [`examples/`](../../examples/)、[README](../../README.md#use-it-as-a-rust-sdk-in-process) 的 SDK 章节 |
| 查看完整的 HTTP/SSE API | [`docs/guides/API.md`](./API.md) |
| 跨破坏性变更升级 | [`docs/guides/MIGRATION_GUIDE.md`](./MIGRATION_GUIDE.md) |
| 了解 crate 布局/架构 | [`docs/README.md`](../README.md) |
