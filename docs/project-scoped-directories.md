# 仓库级 Bamboo 目录

Bamboo 将绑定仓库的运行时文件保存在 Git 根目录之下。这些文件是当前的 Workspace 覆盖层；它们并不是 [Project 身份与共享资源](project-identity.md) 中描述的一等 Project 主目录。

```text
<git-root>/.bamboo/
├── settings.json
├── settings.local.json
├── worktree/<name>/
└── tmp/subagents/<child-id>/
```

`settings.json` 仍适合纳入版本控制。当 Bamboo 创建项目运行时目录时，它会增量维护 `.bamboo/.gitignore`，写入 `worktree/`、`tmp/` 和 `settings.local.json`；已有的忽略条目会被保留。

受管 worktree 使用仅含 ASCII 字母、数字、`-` 或 `_` 的经过校验的名称，以及 `bamboo/<name>` 分支。如果目标位置或分支已存在，创建会在调用 Git 之前失败。任务结束时所有者应调用移除 API；它会先运行 `git worktree remove --force`，随后运行 `git worktree prune`。基于保留时长的垃圾回收器会回收进程故障后遗留的 worktree。只有当 Bamboo 的所有权标记和目录都已过期，且该检出仍带有精确的 `bamboo/<name>` 分支时，它才会移除该检出。无主、新鲜、游离状态或分支不匹配的目录会被保留。

带有显式 `storage_dir` 的子代理保留该目录。若没有指定，workspace 属于某个 Git 项目的 worker 使用 `.bamboo/tmp/subagents/<child-id>`。无 workspace 的 broker/fabric worker 保留操作系统临时目录回退。项目搜索工具会排除 `.bamboo/worktree/`，避免检出通过其兄弟 worktree 被递归索引。
