# 默认 Supervisor 权限证明（#1324）

固定的默认 Supervisor 在其规范的 `session.json`、`runtime.json` 与 Root 工具证明旁边存储一个有界的 `supervisor-authority.json`。该新文件绑定 Session ID、出生时间、incarnation、Root 工具选择/修订版本，以及完整的有界管理状态（Project 范围、链接/墓碑与修订版本）。它是规范权限围栏，不是目录，也不是第二个管理写入者。管理状态仍保存在 `runtime.json` 中。

## 运营读取

`load_root_authority`、Supervisor 范围/链接读取、followup 与管理变更都会获取既有的 lifecycle、Task 与 Session 锁。它们解析 `runtime.json`，检查物理 Root 位置与常规 main 文件，然后要求与该运行时快照匹配的已提交 Root 工具证明与 Supervisor 证明。它们不读取 `session.json` 中的对话字节。同 ID 重建之后，用持久化的吊销截止时间与已证明的出生时间比较，而不是扫描 main 会话记录。完整 Session 加载与完整保存仍会解析 `session.json`，并对照 sidecar 校验其身份与管理覆盖层。

## 发布与恢复

- Bootstrap 在发布目录与索引之前，先暂存完整的 main/runtime/证明集合。复制 Supervisor 仍会创建 Ordinary Root，绝不复制 Supervisor 权限。
- 管理变更先持久化发布 `Prepared` 证明，然后是新的 `runtime.json`，最后是 `Committed` 证明。完整保存会将该顺序与 Root 工具证明协调，并在提交两份证明之前写入 main。任何被中断的中间阶段都会拒绝运营权限。如果最终提交已成功但确认失败，重启后的读取会观察到已提交的修订版本；调用方应在重试之前重新加载。
- 既有的 Task journal 只变更 Task 字段。启动会在 Task 恢复之前执行一次性的 Supervisor 证明升级，然后在发布持久迁移标记之前检查新恢复出的配对。此后，缺失、损坏、pending 或过期的证明绝不会被重新生成。旧配对只有在完整解析 main/runtime、校验 Supervisor 覆盖层并通过既有 Root 证明校验之后才会升级。旧运行时管理修订版本领先于 main 时，只要满足覆盖层规则即有效，因为历史上管理更新曾单独写入 sidecar。

证明上限为 256 KiB；管理容量最多为 64 个 Project 和 256 个链接。其单次操作成本与会话记录长度无关。该协议处理被中断的本地写入与过期的单文件状态。它不宣称能检测所有规范文件与迁移标记被协同回滚的情形。修复被中断的 `Prepared` 管理发布需要基于可信的历史证据进行显式恢复；普通写入者无法猜测或悄悄将其前滚。
