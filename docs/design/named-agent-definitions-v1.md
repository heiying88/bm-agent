# 全局命名代理定义 v1

Issue #1326 在 `bamboo-skills::named_agents` 中新增一个仅宿主使用的解析器/catalog。`NamedAgentCatalog::load_configured()` 读取已稳定的全局 Bamboo 数据目录（`bamboo_config::paths::bamboo_dir()`）及其直接的 `agents/*.md` 子项。`discover(data_root, limits)` 接受一个受信任的宿主配置根。这里没有公开的 HTTP 端点，也不会自动应用到某个 SubAgent。

## 契约

```markdown
---
schema_version: 1
name: rust-reviewer
description: Reviews Rust changes
model_hint: provider:model-1
tools:
  allow: [Read, mcp__files__read]
  deny: [Write]
---
Review the assigned patch and report concrete findings.
```

- UTF-8；LF 或 CRLF；首行与收尾 frontmatter 行都恰为 `---`。其后必须跟非空的系统提示正文。正文两侧空白会被修剪；不做 include，也不做模板展开。
- `schema_version` 是整数 `1`。未知版本会得到一个可查看的 `unsupported_schema_version` 诊断。类型错误、字段缺失/重复、未知字段、YAML 别名/锚点/标签以及畸形 YAML 一律拒绝。
- `name` 精确匹配、大小写敏感，且与文件名无关：1–64 个 ASCII 字节，以小写字母开头，其余为小写字母、数字、`_` 或 `-`。名称既不被规范化，也不设别名。
- `description` 是非空、去除首尾空白、至多 512 个 UTF-8 字节的普通展示字符串。控制字符、换行、双向文本（bidi）嵌入/隔离控制字符以及 `/` 或 `\` 都会被拒绝。因此描述不可能包含物理路径或 URL 目标。
- `model_hint` 可选。它与每个工具标识符都是 1–128 个 ASCII 字节，以字母/数字开头，其余为字母、数字、`_`、`-`、`.` 或 `:`。端点、凭据或文件系统路径都不是模型提示。
- `tools` 可选；`allow`/`deny` 默认为空列表，各自至多 32 个不同标识符。条目重复或两列表重叠无效。这些仅是声明：不授予任何权限，也不解析任何工具。
- frontmatter 至多 16 KiB，文件内容至多 64 KiB，修剪后的提示正文至多 48 KiB。上限按字节计，而非字符数/token 数。

## 发现与资源边界

初版能力读取器支持 **macOS 与 Linux**。Windows 及其他平台返回 `unsupported_platform`，不带任何定义或元数据行；它们不会被报告为畸形。Windows 基于句柄的读取由 #1336 跟踪。本模块不做全平台安全声明。

配置的根必须是绝对路径且不含父目录穿越。从 `/` 起的每个祖先、`agents` 目录以及每个最终候选都以 no-follow 语义打开。每个子项都相对于所保留的父目录描述符打开。枚举也绑定到该描述符。不存在"先检查路径、再不加保护地重新打开"的模式。若某个被配置的祖先本身是符号链接（例如 macOS 的 `/var`），请改配其真实物理路径。catalog 不会把符号链接规范化成可接受的位置。

只考虑直接的、大小写敏感的 `.md` 候选。不遍历嵌套树。符号链接文件、目录、FIFO 及其他非常规文件一律拒绝；读取先以非阻塞方式打开，再检查打开后的文件类型。`agents` 目录缺失即为空 catalog；不可用/不安全的全局根是一次类型化的 catalog 拒绝。

硬性上限为 128 个候选、1,024 个已扫描目录条目（含非 Markdown 条目）与 1 MiB 总字节数。调用者只能收紧这些上限，不能放宽。读取同时受文件上限与剩余总预算约束，另多读一个字节专门用于检测溢出。实际读取的字节数都计入，包括畸形/含秘密的来源以及在 stat 之后又变大的文件。声明超大的文件不读内容直接拒绝。总预算同样约束序列化元数据加保留的定义字符串。超出候选、扫描或总量任一上限都会拒绝整个发布；不存在任意的部分胜者，也不保留旧快照。

## 无效、冲突与元数据视图

每个无效候选产生一个带静态诊断码的匿名 `invalid` 行。不包含文件名、路径、解析器错误或来源摘录。所有占用同一重复名称的有效候选都变成带 `duplicate_name` 的 `conflict` 行；其中任何一个都无法通过精确名称查找获得。有效定义持有原始文件字节的 SHA-256 版本号，因此哪怕只是换行符变化也会改变身份。

可序列化的 catalog 视图是 `NamedAgentCatalogMetadata`：安全的名称、描述、版本号、状态与诊断码，外加一个可选的 catalog 诊断码。定义把提示、路由提示与工具声明保留在宿主内存中，没有序列化实现，其 Debug 输出也限制在同样的安全元数据内。catalog 不存储任何物理路径。元数据是内部的检查/展示数据，绝不是面向模型的指令或权限权威。

## 凭据边界与后续工作

解析器会拒绝 frontmatter 或提示赋值中形似凭据的字面字段，如 `api_key`、`client_secret`、`password`、`authorization` 与 `private_key`，可识别的私钥块，以及若干常见 token 家族（OpenAI 风格、GitHub、AWS access ID 与 Slack token）。它拒绝整个定义，且不回显其名称、描述、秘密值或来源。原始来源与每个解码后的 YAML 标量/列表值都会检查，因此引号转义序列绕不过凭据检查。这是一次保守的语法检查，**并不能检测任意散文里嵌入的每一个秘密**。不要把凭据或敏感材料放进这些文件。

Project/插件优先级、最近良好（LKG）重载、profile 应用与宿主权限交集（#909）、精选默认角色（#1315）、UI 与 HTTP 面仍是 #912 之下的独立工作。本切片没有 watcher、磁盘写入器、额外的持久化协议或运行时模型路由决策。
