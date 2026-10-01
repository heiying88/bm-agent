# Skill 与项目指令

Bamboo 在 skill 存储启动或被显式刷新时重新扫描 skill。当同一个 skill id 出现多次时，各来源按以下优先级使用（最高在前）：

1. `<workspace>/.bamboo/skills[-<mode>]`
2. `${BAMBOO_DATA_DIR}/projects/<project-id>/skills[-<mode>]`
3. `~/.bamboo/skills[-<mode>]`
4. `~/.agents/skills/**/SKILL.md`
5. 已安装 plugin 的 skill

特定模式的 Bamboo 目录会覆盖其对应的通用同级目录。同层冲突时保留第一个确定性发现结果并发出警告。无效、不可读或缺失的 skill 会被记录并跳过，不会阻止其他 skill 加载。发现过程从不跟随目录符号链接。

`<workspace>/.bamboo/workflows/*.md` 中的遗留仓库工作流文件会以只读方式被发现，并以 `legacy: true` 和 `migration_status: "available"` 出现在工作流目录中。Plugin 的 `workflows/*.md` 文件原地使用同一个适配器；安装过程从不把它们复制到用户的全局工作流目录。没有描述的遗留文件会获得一个占位描述，并且只能显式/手动调用，绝不会被自动触发。

内置工作流通过 `POST /api/v1/bamboo/workflow-catalog/<id>/clone` 克隆。Bamboo 在同一文件系统上的私有 `.workflow-clone-txn` 目录中写入 bundle，为有界的 `prepared -> stage_bound -> staged -> complete` 转换记录日志，并以原子化的 no-replace 重命名发布它。精确重试会恢复每个已完整记录日志的阶段；歧义的部分状态或身份不匹配会失败关闭（fail closed），不做任何变更。并发目标在 Bamboo 持久记录 `aborted` 后原样获胜。如果已完成的克隆目录随后被删除，Bamboo 会先记录 `retired`，再开始一个有界的替换纪元；公共名称下不同代际的目录永远不会被退役、收编或删除。

迁移是通过 `POST /api/v1/bamboo/workflow-catalog/<id>/migrate` 进行的显式事务，使用受信任的 session id 来解析源和规范目标层。`${BAMBOO_DATA_DIR}/workflows` 下的用户源迁移到 `${BAMBOO_DATA_DIR}/skills`；由已分配 Project 拥有的 Workspace 遗留源迁移到该 Project 的 `skills` 目录。未分配的遗留 session 保留有界的 Workspace `.bamboo/skills` 兼容目标。发布复用与内置克隆相同的私有暂存、fsync 和原子 no-replace 事务。

迁移从不修改或删除源，从不覆盖已存在的目标，精确重复请求返回 `already_migrated`，且不会重写此后的用户编辑。迁移后的 `SKILL.md` 元数据会记录 `original_source`、被接受的源修订/内容摘要，以及 `legacy_source_removal_boundary: lotus-119-complete`。该边界意味着，只有当 `bigduu/Lotus#119` 完成且消费方都已迁移到规范目录后，源兼容路径才可能被移除；此端点不会移除它。可选的请求字段 `description` 会替换占位描述，否则该 bundle 仍保持仅限手动。公共目录报告一个规范的 `migration_status: "migrated"` 条目，只读适配器仅作为被遮蔽的诊断保留，绝不会成为重复行。

[Claude Code Skills 与遗留自定义命令的兼容性](https://code.claude.com/docs/en/slash-commands)仍以 `.claude/skills` 和 `<workspace>/.claude/commands` 为根；Bamboo 不引入也不标记 `.claude/workflows` 约定。仓库 Skills 同样遵循文档化的 [Codex Skill bundle 模型](https://learn.chatgpt.com/docs/build-skills)。

对于仓库指令，Bamboo 会找到最近的 Git workspace 边界，并从该根目录向下读取直至当前工作区目录的所有适用 `AGENTS.md` 和 `CLAUDE.md` 文件。根规则先注入，更深的规则后注入，因此最具体的作用域可以细化仓库默认值。Git workspace 之上的文件和符号链接的指令文件会被忽略。Git worktree 有自己的 `.git` 边界，会收到同样的已检出仓库指令。该上下文由主 agent 和子 agent 共用的运行时提示词路径组装。
