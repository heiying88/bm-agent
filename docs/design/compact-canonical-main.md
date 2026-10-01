# 紧凑规范 Main 生产者（v1）

V2 host 通过**一次 `session.json` 整体替换**同时写入前置 authority 段和既有的平铺 Session。Runtime/proof/Task/Management 各通道保留既有表示形式与发布边界。这是一个 Main 生产者加全量读取者兼容切片。独立的 #1339 Actor 快照观察者只读取前置段，同时保留其 Runtime、proof、行与谱系检查。两个切片都不引入动作授权或 ContextRefs 消费者。

## 固定帧结构与封闭载荷

从第 0 字节起是字面量 `{"_bamboo_main_authority":{"version":1,"payload_bytes":"`，随后恰好是 10 个 ASCII 十进制数字，接着是 `","payload":`、恰好该数字节数的 UTF-8 载荷字节，然后是 `},` 以及去掉首个 `{` 的原始平铺 Session 编码。该数字是字节长度，不是 revision。整段（含帧结构）必须落在 **512 KiB** 以内。全量 history 不受这条新上限约束，既有的全文件/聚合 authority 读取者预算也保持不变。

全部 15 个载荷成员均为必需，即使取值为 null、0 或 false：

- `id`、`created_at`、`kind`、`parent_session_id`、`root_session_id`、`spawn_depth`；
- `authority_identity`、`metadata_version`、`project_id`；
- `root_orchestration_only`、`root_tool_authority_revision`、`root_mode_transition_epoch`、`root_mode_operations`；
- `supervisor_management`、`title_label`。

取值来自既有 writer 已准入的那个确切 typed Session。Project 使用 `Session::project_id_meta()`（先取 typed Some，再退回 legacy map）。原始逻辑 ID、存储中留空的 legacy Root 写法、typed birth 与 identity、既有 receipt/顺序/边界以及管理墓碑记录均原样保留。`title_label` 会移除控制字符与双向控制符，并截取 160 个 Unicode 标量；它只是一个公开标签。history/native 上下文、prompt、Task 状态、权限、Inbox/lease、effort、预算、workspace 或健康信息都不会进入该段。

未知、重复或缺失的载荷成员，畸形的嵌套结构，被折叠去重的重复 Project 集合，不受支持的版本，错误的数值形态，非规范的 envelope 顺序/转义，溢出/截断或错误的分隔符，均视为无效。可空字段也必须显式出现，不得静默取默认值。

## 生产者准备与既有持久化边界

共享的 host 编码器会在指定发布者开始自身新的状态变更之前，先对该段做预检。容量不足时显式拒绝；绝不截断 authority、丢弃 receipt，也不会把 legacy Main 当作发布成功。

- 全量/初始/mode-operation/runtime-first 回退：在 writer 目录准备和 Prepared proof/Runtime/Main 发布之前预检。
- Copy：构造/改写确切的新目标，并在其 copy journal、staging、attachments、Runtime/proofs/Main 之前预检；复用准备好的字节。
- Root 重建与 Supervisor 引导：构造确切的新 birth，并在已吊销目录的移除/staging 之前预检。Main 与 legacy Runtime 使用各自独立的缓冲区；保留成员绝不会出现在 Runtime 中。
- Clear：在启动其 attachment/Main/Runtime 任务之前预检，包括从实际 Runtime overlay 读取更新的 metadata。
- Explicit Actor append：校验并保留确切的现有原始段字节；只有其已支持的 transcript 通道会变化。legacy append 保持 legacy 行为。

既有的 authority 读取、物理 lock-file 获取、构造器工作以及独立的 Task/copy 恢复保持原有顺序，不在该发布者预检保证的范围内。既有的 Runtime-first、Prepared/Committed proof 与分阶段 copy/Root 故障行为保持不变。每个已启动的文件系统任务保留其既有的专属守卫。这不承诺调用方取消或运行时关闭时每个后续异步阶段都能完成，也不新增任何恢复 journal。

## 全量读取者兼容与观察边界

指定的 V2 全量 Main 兼容读取者在执行 overlay/归一化、proof 创建、索引/缓存发布或受保护重建之前，先依据平铺 authority 字段校验已存在的段。兼容检查在完整校验 JSON 语法的同时跳过私有 history 分配；每个原有消费者随后仍保留各自的全量/部分解码器与 authority 校验。它并不声称某个部分读取者会校验它从未读取的私有 message/native 语义。

真正缺失的成员保持既有的 legacy 全量读取者语义/默认值。已存在但畸形、被移动、被转义、重复或自相矛盾的段一律 fail closed；没有读取侧升级、缓存回退或修复。旧的 typed writer 可能丢弃该成员，产生未来 compact-only 消费者无法使用的 legacy 输入。Runtime-only/Task/Management/migration 保留 Main 字节。

纯前缀解码器只能证明它观察到的帧/载荷字节，无法证明未见的后缀存在、语法合法，或与平铺 birth/Project 一致。外部原始 writer 修改平铺 authority 却保留合法前缀的情况，会被全量兼容检查拒绝；仅靠前缀观察无法发现这种情况。这不是 checksum/正文证书，也不是一刀切的混合写入者完整性保证。retained-FD/no-history 消费者（#1339）显式接受这一更窄的公开观察边界；其 Main FD 在该段的确切结束处停止，其附属的完整文件检查保持不变。legacy 缺失形态在那里不受支持，也没有 GET migration 或 full-history 回退。ContextRefs（#1343）仍是独立的验收切片，不能从该视图继承任何动作授权。
