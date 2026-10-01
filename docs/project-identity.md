# Project 身份与共享资源

Bamboo Project 是一个稳定的、用户本地、由各 session 共享的身份。它不是从 workspace、仓库名、远程 URL 或路径哈希推导出来的。当 session 的当前 workspace 在主检出、链接 worktree、子目录或未注册的临时目录之间切换时，其 `project_id` 保持不变。

Project 存放在所配置的 Bamboo 数据目录之下：

```text
${BAMBOO_DATA_DIR}/projects/<opaque-project-id>/
├── project.json
├── settings.json
├── skills/
├── skills-<mode>/
├── commands/
├── memory/v1/
├── artifacts/
└── state/
```

`project.json` 是权威。`projects/index.json` 可以重建。Project 更新使用 revision/ETag 的比较并交换。目录名使用的是不透明的 ID 而非显示名，因此重命名 Project 不会移动其资源。归档的 Project 保留其 session 和资源。

## Project 与 Workspace

每个新的活跃 Project 都有一个规范的 `project_path`：用户的源码目录，也是 Project 的默认执行目录。它与 `project_home`（即上文展示的 Bamboo 私有资源目录）不同。Project 还可以注册额外的 workspace/worktree 根。所有根都拥有各自的后代路径。当经边界约束解析出的目的地属于另一个 Project 时，Bamboo 会拒绝该 Project 路径 CAS 更新、session 分配或 Workspace 工具变更。未注册的目录仍是临时 workspace，不会被加入注册表。对于已分配的 session，用户通过 session PATCH 或聊天发起的显式切换会以 `project_workspace_unbound` 拒绝未注册目录；请先通过 Project workspace API 绑定它。未分配的 session 则可以继续选择未注册的目录。

已分配的 session 按唯一的优先级顺序解析其生效 workspace：

```text
explicit request/tool workspace
  > persisted session workspace
  > Project.project_path
```

解析到此为止。全局 `default_work_area` 和 session 作用域的临时目录只是面向未分配/旧版 session 的兼容回退。缺失、被移动、不是目录或因边界约束被迁移的 Project 路径会以 `project_path_missing` 或 `project_path_unavailable` 失败（fail closed）；它们绝不会漂移到全局目录、外部 Project 目录或临时目录。

资源优先级为：

```text
builtin < global/user < Project home < current Workspace < session activation
```

workspace 局部的 overlay 仍位于 `<git-root>/.bamboo/`。Project 的 skill 与命令由该 Project 的所有 session 共享；Workspace 的 skill 与命令可以覆盖它们。确定性工作流仍是 skill 包内的 `workflow.yaml` 文件——不存在独立的 Project `workflows/` 目录。

仅为兼容既有仓库，Workspace 可以包含旧版只读的 `.bamboo/workflows/*.md` 来源。已分配的 Project session 会把这样的来源显式迁移到 Project home 规范的 `skills/` 层；未分配的旧版 session 则保留有界的 Workspace `.bamboo/skills/` 回退。迁移绝不会创建 Project home 的 `workflows/` 目录，也绝不修改来源，并且会在迁移后的包中记录相对路径 `original_source` 以及 `lotus-119-complete` 兼容性移除边界。

Project 记忆与 Dream 数据存放在 `projects/<id>/memory/v1`。已分配的 session 绝不会从 workspace 路径推导写入作用域。

## API 与传播

`/api/v1/projects` API 负责创建、列出、更新、绑定、解绑、查看、归档和取消归档 Project。创建时必须提供 `project_path`；list/detail 会返回它；PATCH 可以在不改变 `project_id` 的情况下修改它。变更操作需要通过 `If-Match` 提供当前 revision。`POST /api/v1/projects/{id}/unarchive` 只恢复已归档的 Project，返回其规范清单和新的 ETag，并发布 `ProjectUpdated`。缺失或过期的 `If-Match` 值返回 `428` 或 `412`；恢复一个本就处于活跃状态的 Project 会返回结构化的 `project_not_archived`（`409`）。恢复会保留 Project 身份、路径、绑定、共享资源和 session 归属。当前主路径不能被解绑——请先用 Project CAS 选定替代路径。session 的 create/list/detail 与聊天契约都会暴露 `project_id`；显式的 session 重新分配同样要求 `If-Match`，且在该 session 运行期间会被拒绝。

`PATCH /api/v1/sessions/{id}` 接受 `workspace_path`，作为即时持久的 Workspace 切换。它要求 `If-Match`（缺失时 `428`，过期时 `412`），绝不改变 `project_id`，在返回前更新 session 元数据/索引，并在账户 feed 上发布携带 `project_id`、`workspace_path` 和 `metadata_version` 的 `session_project_updated`。已分配的 session 只能选择已绑定到其活跃 Project 的既有路径；该端点绝不会顺带绑定路径。错误以结构化形式给出：`workspace_invalid`（`400`）、`project_workspace_conflict`、`project_workspace_unbound`、`project_archived` 或 `session_project_running_conflict`（`409`）。同时发送 `project_id` 与 `workspace_path` 会作为一个显式重新分配事务进行校验并提交。

`POST /chat` 保留其兼容行为：显式提供的 `workspace_path` 会成为该聊天轮次的持久 Workspace。它使用与 PATCH 切换相同的活跃 Project、路径、归属和既有绑定校验，而省略该字段的聊天请求则保持当前的持久 Workspace/回退行为。

子级、常驻、guardian、远程 actor、调度、connect、无头、TUI 和 SDK 等创建路径都会传播类型化的 Project ID。普通聊天与 Workspace 变更绝不会重新分配它。

系统 prompt 使用相互独立的 Project 与 Workspace 标记块。Project 块区分 `Project path` 与 `Project home (Bamboo data)`；Workspace 块报告生效路径及其来源（`explicit`、`session` 或 `project_default`）。Project 身份/路径保持稳定，而普通的 Workspace 变更只替换 Workspace 块。资源数量与 revision 是按轮次动态变化的上下文，不属于可缓存的身份前缀。prompt 与资源 API 只暴露脱敏后的名称、状态、数量和 revision——绝不暴露 MCP 头、环境变量值或凭据密钥。

## 旧版 Project 分配

Root 持久化借助既有的 `metadata_version` 和 `created_at` 保护 Project 归属。完整写入与运行时写入都会在跨进程 session 锁下重新校验那份小型的规范 `runtime.json`。修订号更旧、创建时间已变化，或 Project 不同却未恰好对应下一个修订号，都会在文件或缓存发布之前返回类型化的存储冲突。Project 变更使用带检查的修订号递增（包括移除与重新分配）；溢出时 Project/Workspace PATCH 返回 `409 session_metadata_revision_exhausted`。在最终写入者处捕获到的并发上下文变更返回 `409 session_authority_conflict`，重试前需要重新加载。

合并保存会把持久的 Project 归属连同其修订号与 workspace 上下文采纳进调用方快照。SDK 首次为已持久化的未分配 Root 分配 Project 时，会校验候选值而不发布运行时 workspace，推进修订号、提交其边车，然后才进入正常的执行准备。新的未保存 session 保留修订号零。这些是面向受信任存储写入者的过期快照保证，而不是面向故意伪造更新修订号的任意 Rust 调用方的 ACL。

运行时状态缺失或损坏的 Root 仍可通过旧版历史回退读取，但自动边车迁移或 Task 写入无法保存、清除或重建它。仅有 main 文件的旧版 Root 与丢失了较新 Project 上下文的 Root 无法区分。必须先恢复其正确的规范运行时状态才能变更；从旧历史重建权威是被有意禁止的。严格的 Root 权威读取还要求 main 与 runtime 的创建时间一致。旧版 Child 边车迁移仍然可用。没有新增身份注册表、Project 纪元或恢复日志。首次创建被中断、只留下空布局目录的情况可以重试。当有效的 runtime 仍能证明完全相同的创建身份、Project 和修订号时，完整保存也可以补全缺失的 main 文件；但该修复过程不能推进上下文。运行时写入与 Task 写入要求两者齐备。

迁移试运行只匹配精确的规范绑定，或一个能安全解析出的公共 Git 目录。歧义名称、缺失路径、远程 URL 和路径哈希保持未分配状态。当试运行 session 只提供 `workspace_path` 时，服务器会读取该既有 Workspace 以推导其规范路径和 Git 公共目录。调用方提供的证据始终是权威的，绝不会被改写。缺失、不可读或不存在的 Workspace 会产生诊断信息而不派生任何证据，而不是让请求失败。这一信息补全是只读的，绝不会更新 session、Project 清单或索引。

清单 v1 迁移只有在恰好存在一个旧绑定时才会提升它。零个绑定保持 `needs_configuration`；多个绑定保持 `needs_selection`，且绝不会按向量顺序或 `main` 标签来消解。
