# 操作指南：安装并信任插件

Bamboo **插件** 是把 MCP 服务器、prompt 预设、skill 和/或工作流打成一个包，作为一个单元安装并注册进正在运行的 `bamboo serve` 实例。[Nova](https://github.com/bigduu/Nova)（macOS/Windows 桌面控制）是参考级的第一方插件。

所有 `bamboo plugin` 子命令都只是对**正在运行的服务器**上 `/api/v1/plugins` 的一层薄 CLI 封装——请先启动 `bamboo serve`。

## 安装

```bash
# 从本地目录安装（开发）
bamboo plugin install ./my-plugin

# 从打包的归档安装
bamboo plugin install ./my-plugin.tar.gz

# 官方 nova 插件，直接从其受信任的 GitHub release 安装——
# 只要 release 由 nova 官方密钥签名，就无需任何 flag
# （主机和该密钥默认都受信任；参见配置参考中的
# `plugin_trust`）
bamboo plugin install https://github.com/bigduu/Nova/releases/download/v0.2.0/nova-plugin-v0.2.0.tar.gz
```

如果插件 id 已安装，`install` 会失败——这种情况请用 [`update`](#update)。本地来源（`local_dir`/`local_archive`）无条件安装；`url` 来源则要走下面的信任模型。

## `url` 来源的信任模型

从 URL 拉取的插件要过**三层相互独立、层层叠加**的检查，默认即安全：

1. **主机允许列表**——URL 的 host+path 必须匹配 `plugin_trust.trusted_hosts`
   （config.json；默认为 `github.com/bigduu/`）中的一项，除非你传
   `--allow-untrusted-host`。
2. **签名**——bundle 的 `<url>.sig` 必须能用 `plugin_trust.trusted_keys`
   中的某个密钥验签（默认信任 nova 和 magpie 的官方签名密钥），除非你传
   `--allow-unsigned`。
3. **校验和**——`--sha256 <hex>` 固定所下载的 bundle。没有它，`url` 安装
   会被拒绝，除非你传 `--allow-unverified`——**或者** bundle 已经通过了
   第 2 层（验签通过的签名比手工粘贴的校验和是更强的保证，因此单凭它
   就能满足这一层）。

净效果：`bamboo plugin install <官方 nova release URL>` **完全不需要任何 flag**。从其他任何地方安装都需要显式给出对应的豁免项：

```bash
# 主机不受信任，但你固定了校验和、并且信任这台主机本身
bamboo plugin install https://example.com/my-plugin.tar.gz \
  --sha256 3a7bd3e2360a3d29eea436fcfb7e44c735d117c42d1c1835420b6b9942dd4f1 \
  --allow-untrusted-host --allow-unsigned

# 完全不受信任的来源，显式接受风险
bamboo plugin install https://example.com/my-plugin.tar.gz \
  --allow-untrusted-host --allow-unsigned --allow-unverified

# 三个 flag 合一的简写（仅限开发/自托管环境）
bamboo plugin install https://example.com/my-plugin.tar.gz --insecure
```

`--insecure` 只会关掉你没有主动选择的检查——与它一起传的 `--sha256` 仍会被校验（不匹配照样拒绝安装）。每次不安全安装都会记录一条醒目的警告并写入 provenance，可通过 `bamboo plugin list --json` 查看。

对于根本不想传任何 flag 的私有/开发实例，还有一个持久的配置级等价物：

```bash
bamboo config set plugin_trust.enforcement off
```

这会让每次 `url` 安装/更新都表现得像传了 `--insecure` 一样，不需要逐次加 flag。这是一个显式的主动开启（`enforcement` 默认为 `"strict"`），并且在该设置生效期间，服务器每次启动都会记录一条警告。

## 列出、更新、移除

```bash
# 已安装了什么——id、版本、状态、已注册能力计数、来源
bamboo plugin list
bamboo plugin list --json

# 把已安装的插件升级到新版本（来源/信任 flag 与 install 相同）
bamboo plugin update nova https://github.com/bigduu/Nova/releases/download/v0.3.0/nova-plugin-v0.3.0.tar.gz

# 卸载——停止/移除其注册的 MCP 服务器和 prompt 预设，
# 然后删除其插件目录（包括原地的 skill/工作流）。
# 除非 --yes，否则会先确认。
bamboo plugin remove nova
```

`update` 在注册新能力集之前，会先丢弃新版本不再声明的能力——升级移除了某项能力后，旧版本不会留下任何残余。

## 安装之后

插件注册的 MCP 服务器/prompt 预设/skill/工作流立即生效——无需重启。skill 和旧式工作流 markdown 会在插件目录下原地发现；工作流文件从不复制进 `~/.bamboo/workflows`。用 `bamboo plugin list --json`（`registered.mcp_server_ids`、`preset_ids`、`skill_dirs` 字段）查看已注册的共享能力，或者专门用 `bamboo mcp status` 查看 MCP 服务器。`workflow_filenames` 只是为了清理旧安装器写入的 provenance 而保留，对新安装恒为空。

## ToolEvent sink 示例

[`examples/tool-event-recorder`](../../examples/tool-event-recorder/README.md) 是一个可独立构建的原生服务/插件，只消费公开的 ToolEvent 协议。它包含生成的 JSON schema、仅含元数据的 golden 事件、跨平台 bundle 布局、明确的权限升级指引、投递保证与限制，以及生命周期诊断。
