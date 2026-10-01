# ToolEvent 记录器插件示例

这是一个刻意保持小型的 Bamboo 原生服务，把经授权的 `ToolEventV1` 投影记录为 NDJSON。它正常的依赖图止于 `bamboo-plugin-protocol`、`serde` 和 `serde_json`：不导入 server、installer、service manager、router 或工具执行器。

仓库中收录的 [`plugin.json`](plugin.json) 只请求 `metadata`。因此宿主会省略 `tool_name`、`path`、`diff` 和 `content`；[`metadata-only.file-changed.json`](examples/metadata-only.file-changed.json) 这个夹具展示了完整的投递形态。schema 由公开的 `bamboo_plugin_protocol::tool_event_v1_schema()` API 生成，并收录在 [`schema/tool-event-v1.schema.json`](schema/tool-event-v1.schema.json)。

## 构建并组装本地 bundle

在仓库根目录构建独立二进制：

```console
cargo build --release -p bamboo-tool-event-recorder-example
```

把它复制到 `${platform_bin}` 选定的确切原生 bundle 位置：

| 宿主 | bundle 路径 | 仓库测试覆盖 |
|---|---|---|
| macOS | `bin/macos/tool-event-recorder` | 原生子进程与绝对 POSIX 路径 |
| Linux | `bin/linux/tool-event-recorder` | 原生子进程与绝对 POSIX 路径 |
| Windows | `bin/windows/tool-event-recorder.exe` | 原生子进程与绝对盘符路径 |

例如在 Linux 上：

```console
mkdir -p ./bundle/bin/linux
cp target/release/tool-event-recorder ./bundle/bin/linux/tool-event-recorder
cp examples/tool-event-recorder/plugin.json ./bundle/plugin.json
```

安装之前，把记录器的服务配置写到 Bamboo 稳定的按插件配置位置：

```json
{
  "output_path": "/absolute/path/to/tool-events.ndjson",
  "startup_log_path": "/absolute/path/to/recorder-starts.log",
  "startup_delay_ms": 0
}
```

配置路径为
`<BAMBOO_DATA_DIR>/plugin_service_config/tool-event-recorder/config.json`。
Bamboo 通过 `BAMBOO_PLUGIN_SERVICE_CONFIG` 传入该路径；它被刻意设计为在
bundle 升级和卸载后依然保留。然后对运行中的 server 安装本地 bundle：

```console
bamboo plugin install ./bundle
bamboo plugin list --json
```

授权变更是宿主策略，不是清单自我授权。本示例只请求 metadata。想尝试路径投递，可修改一份拷贝的清单，请求 `metadata` 和 `paths`，再发送一个 `event_sink_grants` 显式授予这两个 id 的升级请求。全新安装时省略授权意味着只有 metadata；升级时省略授权则只保留仍被请求的既有授权。新请求的观测能力绝不会被隐式授予。

## 投递契约

- 投递是尽力而为且至多一次，绝非精确一次。入队不等于确认，没有持久化缓冲，Bamboo 也不会重试、重放或补发事件。崩溃/重启可能丢弃尚未完整写入的记录。
- 被同一个存活 sink/进程代次接受的事件保持入队顺序。重启或升级会产生新的服务代次；权限更新会产生新的观测策略代次。跨这些代次边界不承诺顺序。
- 工具执行从不等待本进程或其队列。缓慢/崩溃的记录器只会递增有界诊断计数并可能丢失事件；它无法改变工具结果。
- 本清单使用双事件的 sink 队列和 16,384 字节的事件上限。清单默认是 64 个排队事件，每个 sink 上限 1,024，一个清单中的所有 sink 声明的聚合缓冲总计至多 64 MiB。下游服务输入队列还把单条物理 NDJSON 行（含换行符）限制在 1 MiB。
- 公开协议限制还对 session/root/call id、工具名、路径、投影 diff（4 KiB）、投影 content（8 KiB）以及完整 JSON 事件（16 KiB）设有上限。边界按 UTF-8 线上字节计，完整事件的最终上限在 JSON 转义之后检查。超限事件在服务投递之前即被丢弃。
- metadata 是安全的最小集。即使有显式 `paths` 授权，敏感或词法不安全的路径也会被省略，并替换为稳定的 `sensitive_path` 或 `unsafe_path` 原因；脱敏后的事件不含 diff/content 负载。
- 协议版本 `0` 无效，安装会被拒绝。版本 `1` 使用本目录中的 schema。未来的非零版本仍可安装，但在 v1 宿主上处于不活跃状态，宿主支持之前不会收到事件。

## 诊断

`bamboo plugin list --json` 暴露有界、不含负载的状态：请求与已授予的权限、service/sink 代次、观测策略代次，以及 `delivered`、`queue_full`、`service_down`、`serialization` 和 `oversize` 计数器。`delivered` 指被当前确切服务代次的输入队列接受，不代表记录器已刷写或持久存储该行。`waiting_for_service` 表示进程没有可写的当前代次；不支持的协议会上报不活跃原因。记录器把畸形配置/输入的诊断打印到 stderr，不会回显服务配置或事件负载。

生命周期测试在 macOS、Linux 和 Windows 上使用本包独立构建的记录器二进制。Cargo 通过 `CARGO_BIN_EXE_tool-event-recorder` 把它提供给集成测试；测试把该二进制放到上面所述的原生平台路径，然后覆盖安装、一次真实成功的 `Write`、脱敏、队列压力、崩溃/重启、变更权限的升级、卸载以及启动对账。各目标的 CI 是该目标的权威记录；在一台宿主上的运行不能作为其他操作系统的证据。
