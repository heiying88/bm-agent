# 可恢复的 session 创建

`POST /api/v1/sessions` 支持可选的 `Idempotency-Key` 头，客户端因此可以安全地恢复一次结果不明的超时或丢失的响应。省略该头的既有客户端保持旧版的“一请求一 session”行为。

## API 契约

key 必须为 1–128 字节，可包含 ASCII 字母、数字、`-`、`_`、`.` 或 `:`。客户端应当为每个逻辑创建动作生成一个不透明的非机密 key，并将其保留到该动作进入终止状态。

```http
POST /api/v1/sessions
Idempotency-Key: 550e8400-e29b-41d4-a716-446655440000
Content-Type: application/json

{"title":"New session"}
```

首次成功请求返回既有的 `201` 响应：

```json
{"session": {"id": "..."}}
```

等价重放返回 `200`，响应结构相同且 session ID 相同。用同一个 key 携带不同的载荷重放会返回 `409` 与错误码 `idempotency_key_conflict`。指纹涵盖调用方可控的每个 `CreateSessionRequest` 字段（`project_id`、标题生命周期、prompt、model、provider、模型引用、推理力度、Gold 配置与 workspace），使用递归规范化 JSON。服务端默认值与运行时凭据不属于指纹。

在响应不确定之后，查询经过认证的状态端点：

```http
GET /api/v1/session-create-operations/550e8400-e29b-41d4-a716-446655440000
```

对有效的 key，它始终返回 `200`：

```json
{"status":"pending"}
{"status":"succeeded","session":{"id":"..."}}
{"status":"failed","error":{"code":"...","message":"..."}}
{"status":"expired","error":{"code":"idempotency_key_expired","message":"..."}}
{"status":"unknown"}
```

每个有效 key 的状态响应都带有 `Cache-Control: no-store`，因此浏览器或中间方无法固定早期的 `unknown` 或 `pending` 结果。该路由位于正常的 `/api/v1` 访问控制范围之内，不是公开的恢复旁路。

注册表的命名空间是一个 Bamboo 数据目录（当前的本地账户安全域），由该账户的已认证设备共享。它目前不在 key 摘要中编码单个设备或主体。如果将来一个数据目录被互不信任的租户共享，必须先按经过认证的租户/主体身份划分命名空间，使相同的客户端 key 既不会冲突，也不会泄露另一个租户的操作状态。

## 持久恢复与顺序

操作注册表独立于目标 session 目录，位于 `$BAMBOO_DATA_DIR/session-create-operations/v1` 之下。文件名是完整的 SHA-256 key 摘要。记录只包含 key 摘要、规范化载荷摘要、预留的 session UUID、安全的终止错误数据与时间戳，绝不包含原始 key 或请求载荷。

每个新 key 在创建 session 之前先预留并 fsync 一个稳定的 UUID。内存与磁盘上配对的固定锁分片跨并发的 Bamboo 进程串行化同 key 请求，又不会造成无界的锁文件注册表。幂等 POST 把它认领的核心逻辑放到一个分离的 Actix 任务中运行，请求处理器等待该任务的 join handle。因此，丢弃/中止外层请求 future 并不会取消一个已经进入核心的创建。正常完成顺序保持为：

1. 校验 Project/workspace 并准备该 session；
2. 持久保存权威 session，并原子发布可重建的全局 session 索引；
3. 填充内存缓存；
4. 发布运行时 workspace；
5. 把 `SessionCreated` 持久发布到账户日志；
6. 把创建操作标记为成功并返回。

恢复不会只信任可重建的全局索引。它会严格读取预留根 session 的权威 `session.json`；缺失、损坏与不可读是不同的结果。索引修复在一个固定的跨进程文件认领之下变基，并在纠正规范身份/路径的同时保留较新的活跃摘要。随后 SessionRepository 执行其无回归缓存合并。损坏/读取失败返回 `500`，并保持持久回执不变，留待后续修复。

只有当调用者持有同 key 独占认领时，才允许待处理恢复。它可以完成剩余的 workspace 与账户信息流投影，然后把回执标记为成功。状态 GET 使用非阻塞 try-lock：当 POST 持有认领时，它立即报告持久化的 `pending` 状态；当它赢得认领时，会先重读回执再恢复。已成功的 GET/POST 重放只执行严格的权威/索引/缓存对账；它从不再发布 workspace 或 `SessionCreated` 投影，也绝不替换较新的活跃缓存 Arc。

`sessions.json` 的初始化、变更与重置使用固定的索引认领。每次变更都会重读磁盘、应用修改、原子持久化，然后才更新该进程的内存快照。旧/损坏的重建在同一认领之下发布一个可崩溃续跑的标记，然后以短暂的持锁更新进行扫描；生命周期锁与无回归合并防止过期的重建读取覆盖较新的摘要，或复活一个并发的删除。

账户日志的序列号分配与追加同样跨进程串行化。写入者恢复最新的未填满日志，截断撕裂的尾部，追加、刷新并同步文件（创建新文件时还会同步目录）。Session/工作流生命周期事件与 `ConfigChanged` 使用持久的精确一次 ID；配置健康事件只对连续相等的状态去重。确认入队与持久确认共享一个有界截止时间。FTS 索引仍是尽力而为，不属于成功屏障。

没有对应 session 的待处理预留可以用相同的载荷与预留 UUID 重试。带有持久 session 的待处理或已成功预留会被对账为成功。如果已成功的 session 随后被删除，重放返回 `410 session_result_gone`，状态变为终止的 `failed`；在其保留窗口内，同一个 key 绝不会悄悄创建替代品。

## 保留

待处理预留不会过期。让其过期可能在长时间故障之后为同一个逻辑动作分配第二个 UUID。成功与失败的回执在进入终止状态后保留 24 小时。该窗口过后，GET 会报告持久化的 `expired` 墓碑，直到它被物理清理。同 key 的 POST 可以获取认领、移除过期回执并开始一个新的逻辑操作；之后的 GET 观察到的是那个新操作（若删除后尚未复用，则为 `unknown`）。客户端必须在 24 小时内完成超时恢复，并且不得把 key 复用于无关的工作。

启动时，Bamboo 先在不获取认领的情况下识别过期的终止候选。然后使用同 key 的非阻塞 try-lock，跳过繁忙的候选，并在删除之前重读每个已获取的候选，只删除仍然过期的记录。启动从不会等待活动/待处理的工作。待处理与未过期的记录会被保留。损坏的记录会带一条仅含摘要的警告保留，供审慎的手工恢复使用，而不是被悄悄当作过期或删除。

## 可观测性

服务端发出结构化的 `bamboo.session_create` 追踪事件，带有非敏感的 16 个十六进制字符的关联前缀，以及固定的低基数 `phase` 与 `outcome` 字段。阶段覆盖接受、持久预留、保存开始、认领获取、session 提交、完成、重放、状态恢复与处理器终止；耗时/保存时长以毫秒记录，`lock_acquired` 另行记录 `lock_wait_ms`。`response_constructed` 事件只表示处理器产生了 HTTP 结果；它并不声称客户端收到了字节。如果处理器 future 先被丢弃，`handler_dropped`/`cancelled_or_disconnected` 会记录这一有界事实。它可能源自客户端中止或服务端取消，并有意不去猜测是哪一种。幂等创建/恢复路径绝不追踪原始 key、标题、prompt、Gold 配置、workspace 路径/根、provider 与凭据。

这些是结构化追踪，不是持久化的聚合直方图。监控部署可以从固定字段推导出计数器与延迟分布；刻意不为这次 API 正确性变更添加专门的指标库 schema。
