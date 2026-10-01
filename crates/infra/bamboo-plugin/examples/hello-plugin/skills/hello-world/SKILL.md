---
name: hello-world
description: 随 hello-plugin 示例附带的最小参考 skill。用于演示或测试插件系统的原地 skill 发现。
---

# Hello World

这是一个打包在 `hello-plugin` 示例插件
（`crates/infra/bamboo-plugin/examples/hello-plugin/`）内的简单 skill。
它存在的意义纯粹是端到端地走一遍插件系统的 schema：

- `plugin.json` 在 `provides.skills` 下以 `"hello-world"` 声明它。
- 插件一旦安装到 `~/.bamboo/plugins/hello-plugin/`，这个 skill 就会被
  **原地**发现——不复制、不软链接——因为
  `~/.bamboo/plugins/hello-plugin/skills` 会成为一个额外的 skill 发现
  目录（见 `bamboo-skills` 的 `SkillDirectorySource::Plugin`）。

被调用时，向用户打个招呼，并说明你是来自 `hello-plugin` 插件的示例
skill。
