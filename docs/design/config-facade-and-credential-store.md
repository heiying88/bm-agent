# ADR：模块化实时配置与凭据隔离

- 状态：已接受并已实现
- Issue：#597
- 日期：2026-07-21

## 决策

`Config` 仍然是一个兼容/有效值门面（配置门面），而不是持久化聚合体。每个持久化配置节拥有一个版本化 JSON 存储和一个不可变的最近良好（last-known-good）快照。`AtomicJsonStore<T>` 是共享的持久化原语；`LiveSection<T>` 提供带版本号的进程快照与健康状态迁移。运行时组件接收快照/门面依赖，不在请求路径上读文件。

秘密以稳定的 `CredentialRef` 值寻址，且只存储在版本化、加密的 `credentials.json`、环境变量或明确记录在案的外部安全存储中。各配置节的 DTO 可以包含 `credential_ref`、`token_env`、证书/密钥路径与已配置的元数据，但绝不包含明文、密文或 UI 掩码。

## 最终的配置节与文件映射

| 配置节 | 目标文件 | 内容 |
| --- | --- | --- |
| core/network | `core.json` | 服务端 bind/port、代理引用、headless 模式 |
| providers | `providers.json` | provider 实例、路由、默认值/特性、凭据引用 |
| MCP | `mcp.json` | 服务端 transport/设置与凭据引用 |
| tools/skills | `tools-skills.json` | 工具策略、skill 策略与 catalog 设置 |
| memory | `memory.json` | 记忆与后台维护设置 |
| subagents/broker metadata | `subagents.json` | 限额、broker 发现/放置元数据；不含 bearer token |
| notifications | `notifications.json` | 通道设置与凭据引用 |
| connect | `connect.json` | 聊天平台设置、白名单与凭据引用 |
| cluster fabric | `cluster-fabric.json` | 主机、路径与凭据引用 |
| environment variables | `env.json` | 非秘密值；秘密条目携带凭据引用 |
| access control | `access-control.json` | 仅启用状态与凭据引用/已配置元数据 |
| hooks | `hooks.json` | hook 定义与策略 |
| keyword/model mappings | `model-policy.json` | 掩蔽与 Anthropic/Gemini 映射 |
| model limits | `model_limits.json` | 既有的模型限额记录 |
| credentials | `credentials.json` | 仅加密、版本化的凭据记录 |

迁移期间，`config.json`、`broker.json`、`settings.json`、`connect.json` 以及已抽取的 sidecar 仍可读取。既有的 `memory.json`、`subagents.json` 与 `providers.json` 写入现在共用 `AtomicFileStore`；在 manifest 迁移把它们切换到带版本号的信封之前，其线上格式保持不变。

## 快照生命周期与优先级

优先级顺序为：默认值、已迁移的旧值、配置节文件、环境变量，最后是显式 CLI 覆盖。环境与 CLI 层是有效值视图，不会回写。每个配置节快照记录 `revision`、`loaded_at`、来源路径/种类、状态以及脱敏的 `last_error`。

父目录监视器（watcher）是非递归的，接受 create/modify/rename/delete 事件，过滤存储的临时/锁/备份/隔离文件，并合并突发风暴。只有当观察到的字节与已提交指纹一致时才抑制自身写入，因此随后的外部编辑不会被丢弃。候选内容在发布前先经解析与校验。无效输入保留先前的数据与版本号，只改变健康/错误元数据并发布 `config.invalid`，即便磁盘备份可用也是如此。有效修复发布 `config.recovered`。普通提交/重载发布 `config.changed`。内容有变却未推进版本号的外部编辑，会在存储锁下被规范成一个新的持久单调版本号再发布，因此更旧的 CAS token 无法将其覆盖。

删除首先被视为 watcher 的瞬态状况。服务端适配器必须跨去抖窗口重试，之后才应用各配置节的缺失文件策略。Provider、MCP、环境、通知与集群的副作用在快照发布之后运行；适配器必须先构造替代运行时，构造失败则保留旧运行时，并将该配置节标记为降级。

## 原子性、CAS 与恢复

共享存储使用同目录 UUID 临时文件、整写、文件 fsync、原子重命名、Unix 上的父目录 fsync、出错清理、轮换且通过 schema 校验的备份，以及一把跨进程咨询锁（advisory lock）。敏感存储在 Unix 上强制目录 `0700`、文件 `0600`。Windows 则对敏感目录、数据文件与锁文件应用受保护的 DACL，仅授予属主与 SYSTEM 完全访问，并采用写透（write-through）替换语义。普通新文件遵循进程 umask，而替换操作保留已有的更严格 Unix 权限位。

要求的顺序是：

1. 解析并校验候选；
2. 获取存储锁并比较 `expected_revision`；
3. 持久提交；
4. 发布不可变快照；
5. 应用运行时副作用。

每个活跃配置节通过一把节内局部操作互斥锁串行化提交与重载操作，覆盖快照读取、存储操作、发布与事件构造。这封住了文件锁释放之后、持久候选发布之前的空窗；只读快照访问不获取该操作锁。

过期的版本号即冲突（HTTP 适配器映射为 `409`）。失败的提交不会改动活跃快照。多配置节的 API 操作必须拆分为相互独立、对用户可见的提交；未来任何需要全有或全无语义的操作都必须使用分阶段的 manifest/journal 事务，而不是顺序重写文件。

对由凭据支撑的模块化配置节而言，其所属配置节信封是唯一的公共 CAS 权威。这适用于 `core` 代理认证、`env`、`notifications`、`connect`、`access-control` 与 `cluster-fabric`。`credentials.json` 的版本号是内部事务成员与诊断健康值，绝不是这些领域表单的前置条件。一次精确事务会比较客户端配置节版本号，把元数据与显式凭据动作一起暂存，并在迁移锁下对无关的凭据存储并发胜者做三方合并。对所属配置节的竞争编辑返回 `409`；无关的凭据编辑不会制造虚假的领域冲突。已提交的配置节信封与匹配的 `config.changed` 事件使用同一版本号，即使只是替换或清除一个秘密也是如此。

主文件损坏时，字节会被复制到唯一命名的隔离区，最新的有效备份作为降级的最近良好状态加载。不会在损坏文件上写入默认值。修复主文件后状态迁移为健康。通过快照/事件暴露的错误只含类别，绝不包含用户数据或路径。重复读取相同的损坏字节时，会在存储锁下比对内容后复用同一隔离区，既避免无限制的重复文件，也不把内容派生值放进隔离区文件名。

## 凭据清单与引用

| 旧版秘密 | 凭据引用约定 |
| --- | --- |
| 内置 provider 的 API key | `provider.<provider>.api_key` |
| provider 实例的 API key | `provider_instance.<id>.api_key` |
| HTTP 代理认证 | `proxy.default.auth` |
| MCP stdio 秘密环境值 | `mcp.<server>.env_<name>` |
| MCP HTTP/SSE 认证头 | `mcp.<server>.header_<name>` |
| 带 `secret=true` 的用户环境条目 | `env.<name>.value` |
| ntfy token | `notification.ntfy.token` |
| Bark 设备密钥 | `notification.bark.device_key` |
| 集群密码/私钥/口令 | `cluster.<node-id>.<field>`；普通配置把这些引用与已配置元数据存入 `cluster_fabric.credential_refs` |
| connect 平台 token/app secret | `connect.<stable-platform-id>.<field>` |
| 外部 broker 的 bearer token | `broker.external.bearer_token` |
| 访问密码/设备 token | `access.root.password_verifier` / `access.<device-id>.device_token_verifier`；普通配置仅存引用/已配置元数据 |
| Copilot 的 GitHub OAuth 访问 token | `copilot.oauth.github_access_token` |
| Copilot 聊天 token 缓存 | `copilot.oauth.chat_config` |

该存储复用 Bamboo 既有的 AES-GCM 加密密钥，并记录密钥版本以便将来轮换。其公开状态 API 只返回引用、配置状态、来源与更新时间。变更操作只有替换与清除；空字符串与掩码不是凭据值。运行时解析返回一个不可序列化、Debug 输出脱敏的包装器。

## 旧版迁移协议

迁移必须幂等，并以 manifest 为关口：

1. 获取迁移/咨询锁，把可解析的旧输入加密成一个仅属主可访问、事务作用域的备份（Unix 上目录 `0700`、文件 `0600`）；完成后同时删除 stage 与备份；
2. 使用相互独立的 provider/MCP/root 与外部 broker 规划器，把每份旧输入按原始 JSON 解析，使扁平化字段与未知字段仍归属于其所属配置节（未分类字段进入 core 配置节的 `extra` map）；畸形的可选 broker 文档不能阻塞主配置；
3. 把凭据清单抽取为加密记录，并用引用/已配置元数据替换普通值；
4. 暂存每个候选文件并校验完整候选集；
5. 对已暂存文件 fsync，然后原子地安装一个版本化 manifest 作为提交点；
6. 重启时，丢弃未提交的 stage 或从 manifest 续跑；绝不因存在某个配置节文件而推断迁移已完成。

Provider、provider 实例、MCP、代理、秘密环境变量、通知、connect、cluster-fabric、access-control 与外部 broker 的凭据都使用这份以 manifest 为关口的共享迁移协议。主配置与 broker 两个规划器在同一把锁下独立运行，暂存并 fsync `credentials.json` 及仅受影响的成员，安装一个 pending manifest 作为提交点，并在运行时读取方使用事务成员之前完成或续跑任何已提交的领域。内置/实例 API key、MCP stdio 环境值、MCP HTTP 头与外部 broker bearer token 都变成稳定的凭据引用；明文与旧密文从普通文档及可解析的 root/broker 备份世代中移除。未知字段仍附在原始 JSON 候选上。并发编辑器/API 写入会在配置节文件锁下比较，并以更高的迁移世代变基，而不是被覆盖。用户手写的凭据始终优先于迁移重放。备份世代按最新优先处理：仅存在于备份中的实例会在该备份被改写之前提交进凭据存储，而已配置的同名引用仍优先于更旧的备份值。无法解析的 root 备份保持原样留待人工恢复，而非破坏性地瞎猜。

更旧的二进制之后可能重写无版本号的 sidecar。迁移会把解析出的旧值与已迁移凭据比较：相等则无操作（no-op），不同则推进存储的迁移世代。这既阻止旧的已提交 stage 重放把凭据回滚，又能接纳真正更新的旧版输入。Pending 或畸形的 manifest 是失效关闭（fail-closed）状态：provider/MCP/broker 加载器、启动健康、watcher 与类型化写入保持当前快照直到恢复完成；它们绝不读取不完整的事务成员。只有 `NotFound` 才表示迁移元数据缺失；权限、目录及其他读取失败都是脱敏的失效关闭错误。在迁移锁下规划新事务之前，Bamboo 仅当孤儿 stage/backup 目录的名字恰是受管前缀加规范 UUID、且没有有效 manifest 或 journal 引用它们时才将其删除。符号链接、非 UUID 名字与被引用的事务绝不会被遍历或删除。

Provider 实例的创建/更新/删除、兼容 PATCH 与 CLI 点路径写入共享同一个精确的 credentials/providers/root 事务。客户端自带的 `credential_ref` 与旧密文字段会被剥离，凭据提交先于活跃发布，且当未被引用的实例秘密可能重新进入 `config.json` 时，普通保存会失效关闭。

ntfy/Bark 的通知更新使用自己独立的 Notifications 配置节与凭据事务作用域。完整的通知子树受版本号保护；已提交的事务会对无关的凭据存储编辑变基，拒绝竞争性的 Notifications 配置节编辑，并在出现不安全的消费者冲突时回滚两个成员。可解析的 root 备份世代只有在其中的秘密已持久进入凭据存储之后才会被清洗。运行时 hydration（把引用解析成实际值）在所配置引用缺失或损坏时失效关闭。`PUT /bamboo/config/notifications` 要求 Notifications 配置节版本号，并接受显式的 `keep | replace | clear` 动作。有界的 root-PATCH 兼容路径同样要求该配置节版本号。两者都拒绝掩码与客户端自带的引用/已配置元数据；GET 与变更响应把精确的类型化配置节信封与不含秘密的凭据状态成对返回。

外部 broker 加载器在读取 `broker.json` 之前先完成/复查迁移，然后通过凭据存储解析 `broker.external.bearer_token`。缺失、损坏或已配置但未绑定的引用会失效关闭并回退到内置 broker，而不是在没有 bearer 的情况下去连接外部端点。通用凭据 status/replace/clear API 保留版本号/CAS 语义；普通 broker 文件保持仅含元数据。

两个历史遗留的 Copilot 明文缓存文件在门面启动时被接管：完整的凭据值先提交进加密存储，然后才移除原样未动的旧文件。这两个边界之间崩溃是可安全重试的，之后所有 Copilot 读写都使用凭据权威。

## 服务端集成状态

服务器为整个进程持有一个 `ConfigFacade`，并监视每个模块化配置节。普通配置节变更发布一个不可变快照，并只更新其对应的活跃有效配置字段。Provider 与 MCP 候选有更强的运行时门禁：provider 候选必须在发布前构造出替代的注册表/默认 provider；MCP 候选则要把每个新增或变更的 client 依次经过连接、初始化与工具发现的暂存验证，之后才替换运行时 map/工具索引条目与有效 MCP 快照。失败会丢弃所有暂存 client、保留仍在工作的运行时与工具别名、保住最近良好版本号、把健康标记为降级并发布 `config.invalid`；修复则发布 `config.recovered`。目录去抖与缺失文件重试覆盖编辑器临时写入/重命名风暴。

通用凭据 metadata/status/replace/clear HTTP 适配器使用加密存储，仅对无主引用应用凭据文档 CAS。处于活跃状态的 proxy、Env、Notifications、Connect、Access Control 与 Cluster 引用会拒绝该通用变更路径，并指向各自的领域事务。响应只含状态元数据与健康信息；冲突返回 HTTP 409。成功的变更通过持久化的账号 feed 发布 `config.changed`，该 feed 同时供给 v2 WebSocket 的 `feed` 通道。

只读的类型化 provider 与 MCP 配置节端点暴露与 watcher 相同的独立 revision/health/source 信封。其 DTO 刻意做成诊断性投影：省略 provider key、密文、请求覆盖与未知 provider 字段，MCP transport 的环境/头只报名称不报值。URL 诊断去除用户信息、查询串与片段；MCP 参数值被省略。类型化的 provider/MCP 变更端点保留凭据引用、拒绝新的内联秘密材料，并持久化仅含元数据的 sidecar。运行时构造从 `CredentialStore` 水合引用；被引用凭据缺失或损坏时，候选被拒绝并伴随脱敏的降级/无效迁移，同时保留最近良好运行时。

类型化配置节 API 为普通非凭据配置节提供 GET 信封与带版本号的 PUT 变更。Provider、MCP、Env、Notifications、Connect、Access Control、Cluster Fabric 与凭据使用专门的、经校验的事务；通用 `PUT /config/sections/{id}` 拒绝这些领域，使其无法成为第二写权威。服务器持有的凭据引用字段被保留，不能通过普通 DTO 替换。兼容写入先对门面投影做预检，若会改动多个配置节则在第一次持久写入之前即被拒绝。`model_limits` 不能与其他配置节合并。旧版全量重置端点在活跃的模块化布局下同样被拒绝，直到它具备可恢复的多文件 manifest；调用方需分别重置各配置节。

领域变更适配器使用显式的秘密意图，不使用掩码：

- Env 对每个秘密条目接受 `credential_change: {action: keep|replace|clear}`；省略时保留文档所述的缺失/非空/空兼容形态。秘密转明文必须显式给出新的明文值。
- Notifications 与 Connect 暴露专门的 PUT/GET 适配器，其变更载荷把元数据与 `credential_change`、`token_change`、`app_secret_change` 分开。
- `POST /bamboo/access/password` 要求 Access Control 配置节版本号，支持带版本号的密码 `replace` 与 `clear`，并保留配对设备。公开的认证前 Access 状态暴露权威的版本号、健康与来源投影，但省略配置节数据与本地来源路径；受门禁保护的密码变更返回其精确的已提交信封。
- `POST /bamboo/proxy-auth` 要求 Core 配置节版本号，并返回精确的已提交 Core 信封。

每个变更响应都由该精确事务下捕获的运行时/凭据快照构建，不含明文、密文或 UI 掩码。每个凭据字段报告一个显式状态：所属记录可用时为 `configured`，环境来源活跃时为 `from_env`，没有可用绑定值时为 `missing`，已配置元数据无法安全解析时为 `error`。凭据存储的版本号与健康始终是嵌套的诊断信息，绝不成为领域变更的前置条件。

省略或为空的引用元数据会保留既有绑定（清除是单独的凭据操作）；显式的替换引用必须在运行时暂存或持久提交之前能被解析并定位。根 `config.json` 使用仅落盘的 MCP 投影，对由引用支撑的 env/header 字段同时移除水合明文与旧密文。公开的兼容序列化继续往返（round-trip）水合后的 MCP 形状。

代理认证现在遵循同样的隔离存储边界。旧版 `proxy_auth_encrypted`、按 scheme 加密的字段以及任何旧版内联 `proxy_auth` 对象，都通过可恢复的凭据/配置 manifest 迁移到 `proxy.default.auth`。普通根配置与轮换备份只保留 `proxy_auth_credential_ref`；运行时构造在迁移就绪之后定位并解析该凭据。专门的设置/清除端点使用精确的 Core 配置节事务，其状态与变更响应返回类型化的 Core 信封外加凭据状态，不含用户名、密码、密文或掩码值。普通根配置保存会拒绝未隔离的代理秘密，而不是重新制造旧密文。
