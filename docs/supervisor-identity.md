# 受信任的默认 Supervisor 身份

Bamboo 提供一个受信任的宿主/SDK 服务，在每个本地数据域中维护一个稳定的 Supervisor Root。它确立身份，并管理到独立 Ordinary Root 的、显式限定范围的链接。跟进与取消等命令仍是独立能力，由 #1051–#1058 跟踪。

```rust,no_run
use bamboo_sdk::Agent;

async fn supervisor(agent: &Agent) -> std::io::Result<()> {
    let identity = agent.supervisor_sessions()
        .get_or_create_default("configured-model")
        .await?;
    println!("{} {}", identity.session_id, identity.incarnation_id);
    Ok(())
}
```

宿主也可以直接构造 `SupervisorSessionService::new(agent.storage().clone())`。两个入口都使用同一个规范 Storage 端口。引导不会调用模型、启动 agent、继承 SDK 的 Project/workspace，也不会替换 Session 缓存条目。`initial_model` 只在首次创建时使用；重复调用会保留既有的模型、历史与其他上下文。

回执只包含 `session_id`、`incarnation_id` 与 `created`。请把 incarnation 与 ID 一并保存：显式删除并重建默认 Root 会产生一个新的 incarnation。回执只是一种观察，不是可转移的、用于检查或控制另一个 Session 的授权。Root 删除还会保留[规范的生命周期吊销证据](root-session-lifetimes.md)，因此在 ID 缺失期间，普通的完整/运行时快照无法恢复已删除的 Root。受信任的 Supervisor 引导在删除之后发布新的 birth。

`Session.authority_identity` 是类型化的 `Ordinary` 或 `Supervisor { incarnation_id }` 值，独立于 Root/Child 种类与原始元数据。旧的已序列化 session 默认为 Ordinary。正常的创建、Chat、元数据 PATCH 与所有 Child 构造器都不分配权威。副本是 Ordinary Root；worker、常驻实例、Guardian 与嵌套子代保持 Ordinary。

保留的 ID 是 `bamboo-default-supervisor`；持有这个字符串并不构成权限。如果某个普通 Session 已经占用它，引导会返回显式冲突并保留该 Session。宿主必须通过其正常的 Session 管理策略解决该冲突；引导绝不会提升或删除既有对话。

冷引导还会检查磁盘上规范的子代放置，因此缺失或陈旧的索引不会让一个已被占用的子代 ID 变为可用。只有在默认 Supervisor Root 不存在时才会扫描 Root 目录；普通的重复调用不会触发扫描。

V2 实现通过一次暂存目录发布来发布完整的 `session.json`/`runtime.json` 配对。既有的生命周期锁、Task 锁与每 Session 跨进程锁把它与普通写入者串行化。session 索引仍然是可重建的；一个完整发布的身份可以修复其缺失的索引条目。没有第二身份注册表、单例指针或恢复日志。

`Storage::load_root_authority` 是严格的 Root 控制面端口。它不返回消息，绝不能被用来替换缓存中的完整对话。发布状态的缺失与损坏是不同的情形：缺失/损坏/不匹配的 Supervisor 权威按失败关闭处理，修复与回写期间也一样。当运行时 sidecar 不可用时，它不会从较旧的 `session.json` 恢复权威。配对校验仍会读取规范 main 文件的字节；控制面返回类型并不承诺部分或恒定大小的磁盘 I/O。保留 Root 之外的普通 session 保持既有的兼容读取。不支持的 Storage 实现对身份与管理端口返回 `ErrorKind::Unsupported`，不会回退到普通的 load/save。

merge/save 在提交之前把持久身份并入同一个 Root（创建时间匹配）的 Ordinary 快照；它不会重新绑定到另一个 Root。最终的完整/运行时写入者以类型化的 `SessionAuthorityConflict` 拒绝不匹配的身份；它绝不在内部副本中悄悄替换身份。这同样会拒绝显式不同的 Supervisor incarnation，因此来自已删除 incarnation 的快照无法覆盖重建后的 Root。如果引导在一次并发的首次保存中获胜，那次过期保存会失败，且不会把被拒绝的身份发布到缓存。无关的 I/O 失败保持既有的运行时发布行为。Task 写入、迁移、清空、复制与恢复都必须遵守同一个权威完整性边界。

## 受信任的 Project 范围与链接

同一个服务还暴露 `inspect_scope`、`configure_project_scope`、`attach`、`detach` 与 `inspect_link`。这些都是受信任的进程内宿主操作；没有模型工具、HTTP 自授权路由、自动范围继承、session 目录导出或跨 Root 历史导出。

```rust,no_run
use bamboo_sdk::{Agent, SupervisorReference};

async fn attach_existing_root(agent: &Agent, target_id: &str) -> std::io::Result<()> {
    let service = agent.supervisor_sessions();
    let identity = service.get_or_create_default("configured-model").await?;
    let supervisor = SupervisorReference::from(&identity);
    let observed = service.inspect_scope(&supervisor).await?;
    // 宿主根据其受信任的授权策略选择这组完整集合。
    let projects = ["host-authorized-project".parse().expect("valid Project ID")].into();
    let configured = service.configure_project_scope(
        &supervisor, observed.state_revision, projects,
    ).await?;
    let attached = service.attach(&supervisor, configured.state_revision, target_id).await?;
    let observation = service.inspect_link(&supervisor, target_id).await?;
    assert!(observation.authorized);
    service.detach(&supervisor, attached.state_revision, target_id).await?;
    Ok(())
}
```

范围默认为空，既有的 Supervisor 也不例外。Supervisor 自身的 Project、标签、原始元数据、workspace 路径以及调用者提供的 Session ID 从不授予 Project 访问。宿主范围至多接受 64 个类型化的 Project ID，使用既有 Project 解析器的 64 字节上限与路径安全字母表。

`Session.supervisor_management` 独立于 `authority_identity`。其 schema 版本为 1，其持久化的 incarnation 必须与规范 Supervisor 匹配。字段缺失表示范围为空、没有链接且修订为零；已持久化状态的修订为正数。只有管理 CAS 端口会更改它。普通的构造器与副本没有该状态，子代也从不继承它。规范的 Supervisor `runtime.json` 拥有更新；之后的完整保存可以把同一状态检查点写入 `session.json`。没有关系注册表，也没有目标侧授权。

每个链接绑定目标的确切 ID、`created_at`、类型化的 Project 与 `metadata_version`。attach 要求目标当前是配置 Project 集合中一个完整的、独立的 Ordinary Root。它会原样保留目标两侧的文件，包括身份、世系、Project、workspace、模型、权限与历史。Project 的 A→B→A 变更或删除/重建会使授权失效。无关的元数据修订变更也会保守地使其失效：宿主必须检查当前状态，并再次显式 attach 以重新验证该目标。

状态修订与每个链接自身的修订都会在变更时递增。detach 禁用一个链接，但保留其墓碑。移除某个 Project 会禁用其全部启用的链接；重新授予该 Project 也不会让它们复活。显式 attach 会以下一个链接修订创建全新的绑定。每个 incarnation 至多保留 256 个链接条目，**包括已禁用的墓碑**。条目从不逐出；达到容量时可以重新 attach 既有条目，但另一个目标 ID 会被拒绝。目标 ID 同样有 256 字节上限，并遵守存储的路径安全规则。未知的 schema 版本、畸形的身份、无效的范围/链接状态，以及回退或分叉的覆盖层，都会在权威使用或发布之前失败关闭。

每次变更都要求一个期望的状态修订。两个独立存储以相同的修订竞争时，不可能都改变状态。过期请求会返回 `ErrorKind::WouldBlock`，即使其想要的结果此后已经达成。调用者必须重新加载范围，并用新的修订显式再次调用该操作；一个已经满足的新请求会返回 `changed: false` 而不写入。这是幂等的期望状态行为，不是精确一次的命令去重。任一 `u64` 计数器耗尽时，任何必需的递增都会在发布之前被以 `InvalidInput` 拒绝；一个已满足的无操作仍然可以成功。

V2 会获取生命周期共享锁、Task 共享锁与按字典序排列的 Session 文件锁。它私下重新加载规范的 Supervisor 身份/状态，并在 attach 或观察已启用链接时重新加载目标记录。这些锁会一直持有到持久的原子 Supervisor sidecar 替换完成。该加锁路径内部不会调用任何公共加载器。detach 与范围撤销只需要已验证的 Supervisor 状态，因此目标缺失或损坏不会阻碍撤销。已禁用或缺失的链接返回 `authorized: false`，不要求目标权威；已启用但损坏的目标权威返回错误，绝不返回授权。

普通的完整/运行时写入者拒绝任何管理状态不匹配。既有的 merge/save 路径只有在相同 Root birth 与相同 Supervisor incarnation 时，才把规范状态并入调用者自己的快照。当管理状态在其最终写入者 fence 之前发生变化时，Task CAS 也会拒绝已暂存的观察：不会运行任何 Task 提交或暂存发布回调。条件包装器返回 false；无条件包装器返回 `WouldBlock`。调用者可以发起一次新的调用来重试。这保护了公开的暂存回调观察；生产仓库的 Task 回调仍然只修补 Task 字段。

回执与观察只包含有界的身份/修订/链接字段，绝不包含一个可以装入完整对话缓存的无历史 Session。`inspect_link` 只在其锁持有期间证明授权。返回的布尔值是一种观察，不是留待后续命令使用的授权。未来的 #1051 命令准入必须通过持久 inbox 接受保留最终的关系/目标授权 fence；该链接 API 并不解决之后的那个竞争。跟进、取消、Tracker 订阅与 Plan 委托仍然需要各自的受信任准入检查。与既有的 Session 存储一样，它不是防御同一 OS 用户任意更改数据目录的 OS 沙箱。
