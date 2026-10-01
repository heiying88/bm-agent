# 授权的 Actor subtree snapshot v1

Issue #1337 是 #928 / #791 的第一个只读交付切片。规范网关路由是 `GET /api/v1/actors/{root_id}/snapshot?subtree_id={actor_id}`。省略 `subtree_id` 时选择整棵 Root 树。这些 ID 只用于选择数据。

## Authority 与一致性

浏览器网关要求既有的 bootstrap 认证结果为 `LocalBypass` 或 `Authenticated`（host owner）。在 `Open` 策略下，远程请求在该路由上仍处于未认证状态。设备/cookie 校验与本地性检查沿用既有的网关规则。Codex `bcx1_` 运行 token 保留其仅限 Responses/models 的权限，无法读取该 snapshot。

内部的 `ActorSnapshotPort` 只接受由可信代码构造的 principal（主体），没有 `Deserialize` 实现。存活的 Actor principal 必须证明完整的当前激活围栏、未过期租约、活跃行以及已验证的持久血统。它只能选择自身或其后代。兄弟节点、祖先、外部 Root、单独的 Session ID 或客户端自带的 `requesterId` 都不能授予 authority。在任何子节点枚举之前都会校验完整的存活围栏；发布时还会在同一批已持有的锁下复查完整血统与租约过期。

适配器读取规范的 `sessions/<root>/[children/<actor>/]` 文件。它不使用可重建的索引，也不调用 `inspect_actor`、`ensure_actor`、Session 加载器或自动恢复。它会在整个实际读取期间持有既有的 lifecycle 共享防护与 Task 事务排他防护，包括调用方取消请求之后。存在未完成的 Task 或 copy journal 时，整个视图被拒绝且不做修复。对于被观测的 Main 帧与单独读取的见证，main/runtime 身份、Project、birth、Root 证明、适用时的 Supervisor 证明、每一条直接父边、深度以及 actor 行的祖先观测都必须一致。历史上“actor 行缺失 + 初始化标记”的组合仍视为未知；两者丢失其任一、观测陈旧或身份不一致都会失败关闭。

源 root 及其全部子节点都必须落在预算之内，即使只选择更小的 subtree 也是如此。不会发布部分树。每个子目录都是候选；未知条目会消耗扫描预算，且不能被永远静默跳过。

## 公开传输契约

```json
{
  "schema_version": 1,
  "root_actor_id": "root",
  "subtree_actor_id": "root",
  "snapshot_id": "as1-opaque-equality-digest",
  "stream_cursor": null,
  "nodes": [{
    "actor_id": "root",
    "parent_actor_id": null,
    "root_actor_id": "root",
    "depth": 0,
    "title": "Public Session title",
    "role": "root",
    "logical_state": null,
    "placement_class": null,
    "revision": {
      "session_metadata_version": 0,
      "actor_directory_revision": null
    },
    "activation": null
  }]
}
```

节点按绝对深度与逻辑 ID 排序。公开标题沿用既有的 Session 标题，最多 160 个 Unicode 字符，并移除控制/bidi 格式化字符。Role 只能是 `root` / `child`。已证实的 actor 行提供其逻辑状态、行修订号与可选的安全 activation UUID/attempt/status。对于 host owner 视图，逻辑状态与激活状态是最新持久记录，并不证明某个 Actor 当前在线。心跳与租约活性仍属未知。内部的存活 Actor principal 在获得后代读取 authority 之前，还额外要求未过期租约。放置信息只暴露已证实的激活放置类别，绝不暴露物理主机、端点、放置意图、owner、租约 ID、run ID 或 broker 地址。每 Session 的元数据版本是其持久行值，不是树修订号。

队列/等待/请求/健康信息一概缺席，因为本切片没有它们的权威公开来源。未知的行/状态/放置/激活用 JSON `null` 表示，绝不合成 `cold`、`healthy` 或零值。不序列化任何私有 profile、职责、Project 元数据、环境路径、凭据、prompt、transcript、工具参数或结果。适配器绝不序列化 `ActorDirectoryEntry`。

`snapshot_id` 与 HTTP ETag 是该公开视图的不透明相等身份。`If-None-Match` 可在一次新鲜的授权读取之后产生 304。它们不是有序的全局单调修订号、事件游标、checkpoint 或重放位置。规范的游标/事件投递推迟到 #929 / #930；后续的 Lotus Next 适配器不得把该身份当作流坐标。

## 上限与错误

默认上限：256 个源节点、4096 个累计目录条目、2048 次文件读取尝试（包括不存在的可选文件）、每个源文件 512 KiB、实际读取总量 8 MiB、最终序列化负载 256 KiB。Root 证明与初始化/撤销证据另使用 4 KiB 上限；Supervisor 证明使用 256 KiB。内部调用方只能收紧上限，绝不能放宽。附带的 fd 读取设有上限，并用一个计入的溢出字节检测 stat 之后的增长。最终负载大小包含其身份。ID 至多 256 字节。Main 使用已接受的起始 compact 节：整个节（包括固定帧结构）都必须落在单文件与剩余总量的预算内。完整的 Main 文件可以超过 512 KiB；本观察者从不读取历史。实际的头部与负载字节（包括部分读取）只计一次。已打开的常规文件 FD 止于声明的节边界；Main 关闭后不会再读取后缀、扫描 EOF、探测增长、预读或整缓冲回退。Runtime/证明/行/标记/撤销读取保留其完整文件限制。

## Compact 观测边界（#1339）

该视图校验精确的字面 v1 前缀、十位十进制长度数字、帧结构、封闭的 15 字段类型化负载，以及上文列出的全部可读见证。不支持的旧式、已迁移、转义或未知的起始编码会显式地以 `unsupported_authority` 失败；GET 绝不预备、修复或升级文件。已识别但格式错误/截断的帧结构或负载会拒绝整个视图。既有的完整 Main 读取方仍会校验完整 JSON，并将当前 compact authority 与扁平字段进行匹配。

合法帧加上匹配的 Runtime/证明/行，只是一次公开图观测，并不代表完整的 Main 完整性。该消费者无法检测对该帧及其可读见证不可见的改动：仅存在于扁平字段的 birth 或 Project 修改、后续重复成员、无效后缀、私有 transcript 损坏，或任意重放/篡改。这类文件可以产生相同的公开 snapshot，而完整兼容读取方却会拒绝。snapshot/ETag 不授予任何动作、文件/上下文访问或 ContextRefs authority；#1343 仍是独立契约。

实际的阻塞读取会先持有 lifecycle Shared 防护，再持有 Task Exclusive 防护。一旦启动，调用方中止或整个运行时停机都不会在闭包结束前释放它们。它不保证排队作业一定启动，也不保证被中断的整个异步事务完成。原生 fixture 在只增长私有历史的情况下保持一棵 134 节点的树，追踪实际的 Main FD 偏移，并使用已启动读取方屏障配合独立的物理锁/写者/重开检查。原生执行与确切的工件回执需在源码评审之外单独提供。

类型化的静态错误不包含来源路径或内容：`invalid_selector`（400）、`not_found`（404）、`unauthorized_scope`（403）、`budget_exceeded`（413）、`stale_authority` / `inconsistent_authority` / `pending_transaction`（409）、`unsupported_authority`（501）、`storage_unavailable`（503）。缺少 host 认证为 401。该端点绝不返回部分成功的节点。

初始读取方支持 macOS/Linux，使用能力受限的 `openat`、保留的目录 fd、对每次祖先/最终打开加 `O_NOFOLLOW`、常规文件 fd 检查以及 fd 目录枚举。配置的 home 必须是不含 symlink 祖先的绝对路径。其他平台会在获取锁或读取 authority 之前返回 `UnsupportedAuthority`。Windows 句柄读取在 #1338 中单独跟踪；完整的跨平台交付仍属于 #928 / #791。本切片的原生验证在 macOS 上执行；Linux 运行时验证仍是独立关卡。

不引入前端、实时订阅、重放、历史、ParentRequest/Inbox、远程放置协议，也不引入新的持久全局修订号/journal。
