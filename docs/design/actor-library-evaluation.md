# 面向 #791 的 actor 库决策

决策日期：2026-09-29。

## 当前交付边界

先完成规范的本地 ActorSession、直接父请求、持久的 SessionInbox 准入，以及父/子等待与恢复路径。已支持的固定（pinned）WSS worker 路径保持有界且失败关闭。在持久激活 owner 能够跨进程为 transcript 写入与 Inbox 确认加上围栏之前，自动远程放置、迁移与故障转移仍留在当前交付切片之外。

`ActorSession` 及其 Session 历史仍是持久的 agent 身份。`SessionInbox` 仍是持久的消息 authority。worker、mailbox 地址或库的 actor 引用只是容量或传输手段，不是第二个 agent 身份。

## 评估过的库

| 库 | 有用的能力 | 与 Bamboo 的契合度 |
| --- | --- | --- |
| [Ractor / ractor_cluster](https://docs.rs/ractor_cluster/latest/ractor_cluster/) | 类型化 actor、监督、远程引用与 RPC | 小型执行/监督原型的最佳候选。其[运行时语义](https://github.com/slawlor/ractor/blob/main/docs/runtime-semantics.md)指出重连可能丢消息；仍需要应用层的确认、重试与幂等。 |
| [Coerce](https://docs.rs/coerce/latest/coerce/) | 远程 actor、分片、持久化与 snapshot | 它自带的 ActorSystem 与持久化模型会与 Bamboo 现有的 Session、目录和 Inbox authority 重叠。完整迁移将是一次独立的架构变更。 |
| [Kameo remote](https://docs.rs/kameo/latest/kameo/remote/index.html) | 基于 libp2p 的远程 actor 引用 | 其点对点网络与 Bamboo 由 broker 中介的拓扑不同。 |
| [Actix](https://actix.rs/docs/whatis/) | 进程内 actor 与监督 | 无法替代 Bamboo 的跨进程持久投递。服务端的 `actix-web` 本身并不是 Actix Actor 运行时。 |

这里没有一个库能完整提供 Bamboo 所需的组合：稳定的 Session 身份、直接父 authority、先 checkpoint 后 ack 的准入、Project 边界，以及跨进程激活围栏。现在就替换 broker 或 Inbox，需要重新证明这些契约，并会拖延本地 #791 路径。

如果之后的原型采纳某个库，应把它放在执行或监督的接缝之后，并让 Bamboo 的 Session 与 Inbox 仍是仅有的持久 authority。要用同样的重启、重复投递、父请求与 worker 替换测试，把它与既有实现进行对比。
