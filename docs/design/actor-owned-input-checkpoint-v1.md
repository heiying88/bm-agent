# 自有 Actor 输入 checkpoint v1

`FileSessionInbox::checkpoint_actor_input` 是一个显式的纯存储 API，面向已初始化、处于 Running 的普通 Root/Child Actor 以及精确的 v3 物理 cur 认领。Sealed Supervisor/其他 authority、无效来源或不支持的内容一律失败关闭（fail closed）。未启用任何 Actor/准入/provider/worker/过期调用方；其返回值不是执行权限，本 API 也不做 ACK。

New 与 Already 都会先比较完整的当前 messages/native/admission 前缀。响应丢失后带着 precommit 前缀重试时必须重新加载。User 与 Main 游标构成一次原子替换；Runtime、compact authority、native lane、先前的原始条目与无关的控制字节保持不变。

封闭的 `_bamboo_owned_input_checkpoint` User 元数据对象要求 v1、精确的 envelope ID/target/birth、原始的正数 generation、显式可空的原始 intent 以及冻结的策略。私有的匹配器只会在未变更的 Domain 匹配之前，剥离其经过严格校验的保留键。证据缺失/歧义/格式错误或同 ID 冲突时绝不追加重复项。恢复支持原始的物理 Envelope（包括其 presentation/retry 字段），但前提是处于当前的替换 owner 之下；过期的租约身份不作为去重身份。通用的 Domain 匹配器会拒绝带装饰的 User；生产集成需要单独验收。

每个已启动的完整 std 发布/清理作业都会持有实际的 lifecycle → Task → Session → Inbox 进程/FileExt 防护。新鲜的最终 Actor/Inbox 租约检查跟随真实的 BeforeReplace 屏障。重命名之后/回读失败属于 OutcomeUnconfirmed，需要重新加载；调用方取消与整个运行时停机并不保证未启动的异步阶段会执行。这证明的是协作式的当前 V2 物理写入，而不是全局混合二进制互斥或恰好一次的 provider 效应。
