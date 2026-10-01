# P0 接缝分析——provision.rs / launcher.rs / discovery.rs

Crate：`crates/infra/bamboo-subagent`
范围：深入精读 subagent crate 的三个 P0 接缝文件。每一节把类型/字段映射到行为，然后指出坏味道、边缘情况、竞争以及具体行号。

---

## 1. `provision.rs`——一次性引导契约

**目的（模块文档，第 1–12 行）：**父进程决定一切（模型路由、工具策略、存储、凭据）；worker 只负责执行。spec 通过 **stdin 一次性喂入，随后管道关闭**——刻意*不*用 argv（在 `ps` 中可见）或环境变量（会被孙进程继承）。密钥隔离在专用的信封结构中，这样安全方案（代理模式、短时 token）可以演进而不动引导流程。

### 1.1 常量

| 常量 | 值 | 行 | 作用 |
|---|---|---|---|
| `PROVISION_VERSION` | `1` | 19 | 本 crate 写出的当前 schema 版本 |
| `MAX_SPEC_BYTES` | `8 MiB` | 23 | stdin 读取的硬上限；真实 spec 只有几 KB |

### 1.2 `ProvisionSpec`（第 26–70 行）——逐字段解析

派生 `Debug, Clone, PartialEq, Serialize, Deserialize`。

| 字段 | 类型 | Serde | 默认值 | 用途 / 行为 |
|---|---|---|---|---|
| `version` | `u32` | 必填 | 经 `new()` 设为 `PROVISION_VERSION` | schema 版本；**只写入，读取时从不校验**（见坏味道 S1） |
| `identity` | `ChildIdentity` | 必填 | — | 标识这个子代理是谁 |
| `executor` | `ExecutorSpec` | 必填（内部打标） | — | 指定由哪个引擎运行 |
| `fabric_dir` | `String` | 必填 | — | worker 自注册进入的 Tier-1 发现目录 |
| `storage_dir` | `Option<String>` | `default, skip_if_none` | `None` | session/mailbox 文件的隔离存储根 |
| `workspace` | `Option<String>` | `default, skip_if_none` | `None` | actor 文件操作的工作目录（cwd） |
| `model` | `Option<ModelRefSpec>` | `default, skip_if_none` | `None` | 父进程最终解析出的模型（显式指定 > 按类型路由 > 默认值） |
| `disabled_tools` | `Option<Vec<String>>` | `default, skip_if_none` | `None` | 对子代理隐藏的工具名（profile 策略已应用） |
| `limits` | `Limits` | `default` | `Limits::default()` | 时间/轮数预算 |
| `secrets` | `SecretsEnvelope` | `default` | 空信封 | 限定范围的凭据 |
| `reusable` | `bool` | `default` | `false` | 若为 true，worker 服务多次运行（热池）；每次运行仍会从 `messages` 重建一个全新 session |
| `placement` | `Placement` | `default` | `Local` | actor 在哪里运行 |
| `capabilities` | `Capabilities` | `default` | 空 | 由 orchestrator 同步的 MCP/skills；普通 actor 子代理为空 |

### 1.3 `Placement`（第 104–118 行）——借 `serde(default)` 实现前向兼容

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Placement { #[default] Local, Remote { endpoint }, Schedulable { pool } }
```

- **打标枚举**（`kind` 字段，snake_case）→ 序列化为 `{"kind":"local"}`、`{"kind":"remote","endpoint":"…"}`、`{"kind":"schedulable","pool":"…"}`。
- `#[default] Local` 加字段级 `#[serde(default)]`（第 63 行）→ 早于该字段出现的 spec 会反序列化为 `Local`，保持现有行为。由 `missing_optional_fields_default_backward_compat`（第 278 行）和 `placement_defaults_local_and_remote_round_trips`（第 297 行）验证。
- **前向兼容缺口（S2）：**来自更新版本父进程的*未知* `kind` 值（例如 `{"kind":"k8s"}`）是硬性反序列化错误，不会回退到 `Local`。因此模块文档中“旧 worker 读取新 spec”的说法只对*未知字段*成立，对*未知枚举变体*不成立。没有 `#[serde(other)]` 逃生门（带数据的结构体变体也做不到）。

### 1.4 `Capabilities`（第 75–91 行）——能力同步模型

```rust
pub struct Capabilities {
    pub mcp: Option<serde_json::Value>,         // 不透明；worker 自行反序列化为领域 McpConfig
    pub skills_dir: Option<String>,
    pub mcp_proxy: Option<McpProxyConfig>,
}
```

- **同步模型：**orchestrator 把工具集的快照*推送*进 spec；worker 原样加载。**没有增量/差量同步**——每份 spec 都携带完整能力集。空的 `Capabilities`（默认值）意味着内建工具 + 隔离的空 skills 目录，也就是对 actor 子代理零行为变化。
- **`mcp` 是不透明的**（`serde_json::Value`），刻意为之，让这个叶子 crate 不必依赖 `bamboo-domain` 的 `McpConfig`。校验推迟给 worker。
- **`mcp` 与 `mcp_proxy` 互斥**有文档说明（第 88 行：“与 `mcp` 直连同步互斥”），但在本 crate 中**无处强制**（S3）。同时携带两者的 spec 在这里会被接受；只有 worker 知道要拒绝它。

### 1.5 `McpProxyConfig`（第 94–102 行）

`{ orchestrator, endpoint, token }`——worker 用于把 MCP 工具调用代理到 orchestrator 的 broker mailbox id、`wss://` 端点和 bearer token。`token` 是明文密钥随 spec 传输（与“stdin 而非 argv”的思路一致，但注意它会通过 `McpProxyConfig`/`Capabilities`/`ProvisionSpec` 派生的 `Debug` 打印出来——S4：经 `Debug` 泄露凭据）。

### 1.6 `SecretsEnvelope` / `ScopedCredential`（第 163–183 行）——范围化凭据模型

```rust
pub struct SecretsEnvelope { pub provider_credentials: Vec<ScopedCredential> }
pub struct ScopedCredential {
    pub provider: String,            // 路由键：旧式名称（"anthropic"）或实例 uuid
    pub api_key: String,
    pub base_url: Option<String>,
    pub provider_type: Option<String>, // 具体协议；为 None 时回退到 provider 本身
}
```

- **范围原则（第 163 行）：**凭据只限定在*这个子代理恰好需要的范围*，绝不是整个配置。仅驻留内存；worker 不得持久化它们（在别处强制，不在本文件）。
- **Provider 路由：**`provider` 是多态的——要么是旧式名称，要么是实例 id。当它是实例 id 时，由 `provider_type` 区分具体协议；为 `None` 时回退到 `provider` 本身。这是一个干净的双模式路由键。
- **`api_key` 在结构体中是明文**，且结构体派生了 `Debug`（第 171 行）→ **`Debug` 会打印出密钥**（S4，与 McpProxyConfig 同一根因）。本文件中没有任何 `redact` 辅助函数。

### 1.7 `ExecutorSpec`（第 134–143 行）

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutorSpec { Echo, BambooRuntime, CliAdapter { command, args } }
```

- 内部打标，snake_case → `{"kind":"echo"}`、`{"kind":"bamboo_runtime"}`、`{"kind":"cli_adapter","command":"…","args":[…]}`。由 `executor_tags_are_stable`（第 347 行）验证。
- **扩展方式（第 133 行）：**新增一个引擎 = 一个新变体 + worker 中一个工厂分支。干净。
- **与 `Placement` 相同的前向兼容缺口（S2）：**来自更新父进程的未知 `kind` 是硬错误。

### 1.8 `ChildIdentity` / `ModelRefSpec` / `Limits`（第 120–161 行）

- `ChildIdentity { child_id, parent_id?, project_key?, role }`——`role` 默认为 `""`（第 128 行 `#[serde(default)]`），这是一个合法但语义上别扭的“无角色”值（S5：可考虑 `Option<String>`）。
- `ModelRefSpec { provider, model }`——`ProviderModelRef` 的本地镜像；本 crate 保持叶子地位（不依赖 `bamboo-domain`）。
- `Limits { run_timeout_secs?, idle_timeout_secs?, max_rounds? }`——全部可选，全部默认 `None`（无限制）。**此处不做强制**——这些是参考值，由 worker 负责遵守。

### 1.9 `ProvisionSpec::new` 与编解码（第 185–230 行）

- `new()`（第 186 行）设置 `version = PROVISION_VERSION` 并显式填充每个可选/默认字段——良好的防御式风格，没有 `..Default::default()` 的坑。
- `to_json` / `from_json`（第 204–212 行）是薄封装，把 serde 错误映射为 `StoreError::decode`。
- **`read_from_stdin`（第 219–229 行）——引导流程：**
  1. `tokio::io::stdin().take(MAX_SPEC_BYTES).read_to_end(&mut buf)`——把读取限制在 8 MiB。
  2. `String::from_utf8_lossy(&buf)`——有损 UTF-8（S6：静默改动非法字节；恶意/垃圾 stdin 会变成合法但错误的字符串，然后报 serde 错误，而不是清晰的“stdin 非 UTF-8”错误）。
  3. `Self::from_json(text.trim())`——去空白后解析。
  - **防御：**`MAX_SPEC_BYTES` 防止失控的写入方造成 OOM（管道本受信任，但求纵深防御）。
  - **坏味道 S7：**读取在*时间*上没有上限——写入方打开管道却永不关闭时，`read_to_end` 会永远挂起（没有 deadline）。已声明的威胁是失控的写入方；停滞的写入方是未被处理的孪生威胁。

### 1.10 版本兼容——前向 + 后向

| 方向 | 机制 | 证据 |
|---|---|---|
| **前向**（旧 worker，新 spec） | serde 默认忽略未知字段 | `unknown_fields_are_ignored_forward_compat`（第 268 行） |
| **后向**（新 worker，旧 spec） | 每个新增字段都有 `#[serde(default)]` 或 `Default` | `missing_optional_fields_default_backward_compat`（第 278 行） |

- **警示（S1）：**`version` 只被*写入*，读取时从不*检查*。没有 `if spec.version > PROVISION_VERSION { warn }`，也不拒绝。因此 v1 worker 读取 v5 spec 时，会默默把 v1 语义套用在恰好同名的 v5 字段上。版本字段目前只是摆设。

### 1.11 provision.rs——坏味道 / 边缘情况 / 竞争登记表

| ID | 行 | 类别 | 问题 |
|---|---|---|---|
| S1 | 19, 209–212 | 坏味道 | `version` 只写不校验；没有兼容性闸门 |
| S2 | 108–118, 135–143 | 前向兼容缺口 | `Placement`/`ExecutorSpec` 上的未知 `kind` 是硬错误而非回退——与变体新增场景下“旧 worker 读新 spec”的说法矛盾 |
| S3 | 80–90 | 坏味道 | `mcp` ⊕ `mcp_proxy` 互斥有文档但本 crate 不强制 |
| S4 | 26, 94, 171 | 安全 | `ProvisionSpec`、`McpProxyConfig`、`ScopedCredential` 派生 `Debug` → `api_key`/`token` 可被打印。无脱敏 |
| S5 | 128 | 坏味道 | `role: String` 默认 `""`；用 `Option<String>` 更清晰 |
| S6 | 227 | 边缘情况 | `from_utf8_lossy` 静默替换非法字节 → 下游只报含糊的 serde 错误，而不是清晰的“stdin 非 UTF-8”错误 |
| S7 | 222–226 | 边缘情况 | `read_to_end` 无时间上限；停滞的父进程管道会让 worker 永久挂起 |
| S8 | 153–161 | 坏味道 | `Limits` 仅为参考；本 crate 不强制（可以接受——本就是 worker 的职责——但值得记录） |

---

## 2. `launcher.rs`——部署接缝

**目的（第 1–8 行）：**抽象*worker 如何诞生*，让 fleet/runner 永远不用按部署方式分支。Phase 0 只交付 `LocalSubprocessLauncher`；远程/可调度的 launcher 之后再插入同一 trait 之后。

### 2.1 `WorkerLauncher` trait（第 23–26 行）

```rust
#[async_trait]
pub trait WorkerLauncher: Send + Sync {
    async fn launch(&self, spec: &ProvisionSpec, wait: Duration) -> TransportResult<SpawnedChild>;
}
```

- **对象安全：**该 trait 是 `Send + Sync` 且只有一个 `async fn`（由 `async_trait` 脱糖为 `Box<dyn Future + Send>`）。单元测试 `local_subprocess_launcher_is_a_trait_object`（第 58 行）显式断言 `&dyn WorkerLauncher` 可编译，印证了持有 `Arc<dyn WorkerLauncher>` 的设计意图。
- **`launch` 契约：**为 `spec` 启动（或连接）一个 worker，最多等待 `wait` 直到它可达（在发现服务中自注册完成），返回一个 `SpawnedChild`（它拥有进程句柄，drop 时经 `kill_on_drop` 杀掉进程）。超时时，`spawn_worker` 会杀掉进程（见 `fleet.rs` 文档第 41 行）。
- **返回类型不对称（L1）：**`launch` 返回 `SpawnedChild`，一个拥有 `tokio::process::Child` 的结构体。该类型只对*本地*子进程有意义；`RemoteLauncher`（连接 `wss://`）没有可杀的 PID。trait 的返回类型隐式固化了“本地进程”语义，等远程 launcher 落地时将被迫重构（例如改为枚举返回或引入 `WorkerHandle` 抽象）。这是本文件最大的设计风险。
- **`wait: Duration`（L2）：**期限以位置参数 `Duration` 传入，而不是 `Instant` 或取消令牌。无法从外部提前取消一次 launch；调用方只能等到超时。

### 2.2 `LocalSubprocessLauncher`（第 31–50 行）

```rust
pub struct LocalSubprocessLauncher { pub worker_bin: PathBuf, pub worker_args: Vec<String> }
```

- **它是一个字面意义上零行为变化的包装：**`launch`（第 47 行）只做一件事：转发给 `spawn_worker(&self.worker_bin, &self.worker_args, spec, wait)`。全部真实逻辑（创建 `fabric_dir`、编码 spec、spawn、喂 stdin、关闭、轮询注册）都在 `fleet::spawn_worker`（`fleet.rs:45`）。
- **公开字段（L3）：**`worker_bin` 和 `worker_args` 是 `pub` 的，调用方可以在构造后修改。多半是无意的——launcher 在概念上只应配置一次。`pub` 也削弱了约束（例如没有什么能阻止把 `worker_bin` 设为空）。
- **没有校验（L4）：**`new` 不检查 `worker_bin` 是否存在、是否可执行；失败被推迟到 `launch` → `spawn_worker` → `Command::spawn` → `TransportError::Io`。

### 2.3 launcher.rs——坏味道 / 边缘情况 / 竞争登记表

| ID | 行 | 类别 | 问题 |
|---|---|---|---|
| L1 | 25 | 设计风险 | 返回类型 `SpawnedChild` 固化了本地子进程语义；远程 launcher 没有 PID——将迫使 trait 重构 |
| L2 | 25 | 坏味道 | `wait: Duration` 无法外部取消；没有 `CancellationToken` |
| L3 | 31–34 | 坏味道 | 公开可变字段；构造时不校验 |
| L4 | 37–42 | 坏味道 | `new` 中不检查 `worker_bin` 是否存在/可执行 |

---

## 3. `discovery.rs`——文件 fabric

**目的（第 1–5 行）：**一个与进程无关、基于文件的 Tier-1 fabric。每个 actor 原子地 `publish` `<dir>/<agent_id>.json`；其他角色通过扫描来 `discover`，并丢弃过期（租约失效）的记录。长期运行的服务型 agent 存在于此；自有子代理使用 Tier-2 注册表。

### 3.1 `Discovery` trait（第 122–129 行）

```rust
#[async_trait]
pub trait Discovery: Send + Sync {
    async fn publish(&self, rec: &AgentRecord) -> Result<()>;
    async fn resolve(&self, agent_id: &str) -> Result<Option<AgentRecord>>;
    async fn discover(&self) -> Result<Vec<AgentRecord>>;
    async fn withdraw(&self, agent_id: &str) -> Result<()>;
    async fn gc(&self) -> Result<usize>;
}
```

- **对象安全**（`Send + Sync`，`async_trait`）——`fabric_is_usable_as_dyn_discovery`（第 233 行）通过 `&dyn Discovery` 驱动它。
- **方法集语义：**
  - `publish`——upsert（原子写）。
  - `resolve(id)`——点查，缺失或过期返回 `None`。
  - `discover`——完整在线列表，按 `agent_id` 排序。
  - `withdraw(id)`——删除（幂等）。
  - `gc`——清扫过期文件，返回删除数量。
- **缺失的操作（D1）：**没有 `list_all`（含过期记录），也没有 `watch`/订阅。想要变更通知的调用方只能轮询 `discover`。

### 3.2 `Fabric` / `FileFabric`（第 17–156 行）

`pub type FileFabric = Fabric;`（第 134 行）——别名点明意图，又不用改动调用点。

`record_path`（第 26 行）：`dir.join(format!("{agent_id}.json"))`。

**`publish`（第 32–36 行）：**`serde_json::to_vec_pretty` → `atomic_write`。原子性来自 `error::atomic_write`（`error.rs:49`）：先写同目录下的 `.<stem>.tmp.<uuid>`，`sync_all`，再 `rename`。临时文件名以 `.` 开头且唯一，因此目录扫描器会跳过它（第 66/102 行 `discover`/`gc` 中的 `.` 前缀过滤与此呼应）。

**`withdraw`（第 39–45 行）：**`remove_file`，把 `NotFound` 视为成功（幂等）。

**`discover` / `discover_as_of`（第 48–77 行）：**
- 读目录；`NotFound` → 空 vec（把缺失的 fabric 当作空，而非错误）。
- 对每个条目：跳过 `.` 开头或非 `.json` 的文件；解析；`lease_expires_at > now` 则保留。
- 按 `agent_id` 排序（结果确定；`discover_as_of` 是可测试的形式）。

**`resolve`（第 80–85 行）：**点读 `<id>.json`；缺失、损坏或过期返回 `None`。

**`gc`（第 88–114 行）：**扫描全部 `.json`；`lease_expires_at <= now` **或**不可读/损坏（第 107 行：`_ => true`）即视为过期；`remove_file` 并计数。不可读的文件按过期处理并清除——对本地 fabric 是好的防御选择。

**`read_record`（第 158–167 行）：**`tokio::fs::read` → `serde_json::from_slice`；JSON 损坏 → `Ok(None)`（不算错误）；`NotFound` → `Ok(None)`；其他 IO → `Err`。

### 3.3 `AgentRecord`（定义于 `proto.rs:11–24`，discovery 全程使用）

```rust
pub struct AgentRecord {
    pub agent_id: String,
    pub role: String,
    pub labels: Vec<String>,            // #[serde(default)]
    pub endpoint: String,               // ws://127.0.0.1:<port>
    pub pid: u32,
    pub version: String,                // #[serde(default)]
    pub started_at: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
}
```

- **存活键：**`lease_expires_at`。一旦 `now > lease_expires_at`，读取方即视记录为过期（`discover`/`resolve` 用严格 `>`；`gc` 用 `<=`）。1 秒边界的不对称是良性的。
- **续约 = 重新 publish 并抬高 `lease_expires_at`**（第 30–31 行文档）——没有单独的 `heartbeat`/`renew` 方法；续约和更新是同一操作。
- **`pid`** 是 `u32`（无 `Option`）——远程 worker 没有本地 PID，该字段却是必填。前向兼容坏味道（D2）。
- **`endpoint`** 是必填 `String`——对服务型 agent（文档所述用途）没问题，但假定了每个被发布的 actor 都可经网络访问。

### 3.4 租约过期与 gc——存活机制如何运作

该模型是**软状态、基于租约**的，没有跨进程锁：

1. 启动时，worker `publish` 自己的记录，`lease_expires_at = now + lease_ttl`。
2. 存活期间，它周期性地重新 `publish` 并抬高过期时间（续约）。
3. 读取方（`discover`/`resolve`）过滤 `lease_expires_at > now` → 崩溃后停止续约的 worker 在一个 TTL 后从结果中消失。
4. `gc` 物理删除 `lease_expires_at <= now`（或损坏）的文件。磁盘回收是惰性的；正确性不依赖 `gc` 运行。

**这本质上是一个最终一致、容忍竞争的设计**——没有互斥锁，这是有意为之。

### 3.5 discovery.rs——坏味道 / 边缘情况 / 竞争登记表

| ID | 行 | 类别 | 问题 |
|---|---|---|---|
| D1 | 122–129 | 坏味道 | 没有 `watch`/订阅；变更检测只能轮询 `discover` |
| D2 | proto.rs:18 | 前向兼容 | `pid: u32` 必填；远程 actor 没有 PID → 将来需要 `Option` 或哨兵值 |
| D3 | 26 | 安全/路径注入 | `record_path` 直接 `format!("{agent_id}.json")`，无任何清洗。含 `/` 或 `..` 的 `agent_id` 会逃出 `dir`。今天 `agent_id` 由父进程控制，但一旦变为用户输入就是路径穿越。加固成本很低（拒绝 `agent_id.contains('/')`，或对 id 做哈希）。 |
| D4 | 32–36, 105–109 | 竞争（良性） | `publish`（临时文件 + 重命名）与 `gc`（删除过期）竞争：gc 可能在 publish 目标文件处理中途 unlink 它。原子重命名意味着最坏情况是 gc 删掉了刚写入尚未重命名的*旧*文件，或 publish 的重命名落在 gc 的 unlink 之后。两者都会在下一个 publish/gc 周期收敛到正确状态。不会损坏，但一条记录可能短暂从 `discover` 中消失。对软状态可以接受。 |
| D5 | 105–108 | 边缘情况 | `gc` 把*任何*读取错误（不只是 JSON 损坏）都当作过期并删除。一个瞬时的权限错误就会静默删除一条存活记录。`read_record` 已经区分 `NotFound` 与真实 IO；`gc` 却把 `Err` 分支一律归为“过期”。可以改为出错时跳过。 |
| D6 | 88, 48 | 边缘情况 | `discover` 与 `gc` 相对目录**不是原子的**——先 `read_dir` 再遍历。扫描期间新增/删除的文件可能看到也可能看不到。对存活检测无妨，但 `gc` 返回的计数只是任意瞬间过期数量的下界。 |
| D7 | 60–74, 96–112 | 性能 | `discover` 与 `gc` 都要全目录扫描 + 逐文件读取。上规模（数千 agent）时每次调用都是 O(n) IO 且无缓存。对今天的本地 fabric 可以接受；网络后端就需要索引了。 |
| D8 | 158–167 | 坏味道 | `read_record` 把损坏的 JSON 吞成 `Ok(None)`——静默不可见。损坏时没有指标/日志（本可帮助排查“我的 agent 为什么消失了”）。 |
| D9 | 26 | 边缘情况 | 在大小写不敏感的文件系统（macOS HFS+/APFS 默认）上，两个 id 仅文件系统不安全字符不同的 agent 可能冲突（例如 `Agent` 与 `agent`）。 |

---

## 4. 跨文件观察

1. **密钥处理一致性（S4）：**`provision.rs` 把明文密钥（`api_key`、`token`）放在派生 `Debug` 的结构体里。引导流程本身是对的（stdin，而非 argv/环境变量），但任何人 `dbg!(spec)` 都会泄露凭据。标准修法是 `redacted` Debug 辅助或手写 `Debug` 实现。这是最值得动手的安全发现。
2. **前向兼容只到*字段*级别（S1/S2）：**模块文档承诺双向兼容，对字段确实成立。但对新枚举变体（`Placement`、`ExecutorSpec`）*不*成立，`version` 字段也只是装饰。要么加 `#[serde(other)]` 式兜底（结构体变体做不到——需要 `Unknown { kind: String, rest: Value }` 这样的全兜底变体），要么收窄文档的说法。
3. **`SpawnedChild` 泄漏（L1）：**`launcher.rs` 从一个为远程后端设计的 trait 返回了本地进程类型。这是 `Placement::Remote` 真正落地时最可能需要返工的接缝。
4. **路径安全（D3）：**`discovery.rs` 信任 `agent_id` 是单个路径分量。今天安全；把这条约束写进文档或做清洗，因为这是成本最低的加固。
5. **引导读取无时间上限（S7）：**`read_from_stdin` 有字节上限但没有 deadline。写到一半停滞的父进程会把 worker 挂死。给 `take(MAX_SPEC_BYTES)` 配上 `tokio::time::timeout`。
