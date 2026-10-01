# 自有本地 Actor 修正

显式的 Ultra Root + 全新本地 Bamboo 零工具命名 profile 路由，可以在其当前 Actor 激活期间消费一条有界文本修正。由正常的父级 SubAgent `send_message` 工具投递真正的 Host Inbox 条目；任何 worker ID 或请求元数据都不会成为 authority。

在第一个完成的普通 worker Terminal 上，Host 会认领实际待处理的自有 Inbox 条目，然后使用既有的带围栏 transcript 追加提交该 Assistant。接着使用 `ActorInputCheckpoint` 原子地提交类型化的 User 与准入游标。被拒绝或未确认的 checkpoint 不会派发第二次 Run、不会调用 provider、不会 ACK，也不会回退到普通保存。

第二次 Run 携带已提交的前缀与类型化的初始投递。其既有的已检查 worker 启动准入会在 Host 自有 ACK 之前，确认精确的 target、envelope、generation 与 Host run。Broker 的 Run 关联加上新的原生执行 epoch 会拒绝旧帧；公开 Host feed 的顺序保持不变。同一个 Actor 围栏、attempt 与原始租约/看门狗继续有效。不会有第二次 Actor 认领/启动，也不会有提前的父级成功。

在派发第二次 Run 之前，会从实际的 Host checkpoint 回读刷新其权限审计见证。姿态（posture）与当前 Host 策略仍必须匹配原始上限；一旦变化即拒绝继续。既有的持久审计比对在第二次 worker bootstrap 中仍然有效。

只支持一次续跑（两次 worker Run）。文本限制为 8 KiB，不含 parts。输入租约的过期时间绝不超过原始 Actor 租约，也不超过其初始认领之后一小时。直接的旧式 WS 续跑、原始 steering、工具、推理、远程/嵌套激活以及 Succeeded Actor 重启仍不受支持。稍后的终态竞争或额外的排队输入可以保持未确认；持久的历史/队列会被保留。不引入续租/回收、自动恢复、worker 释放或 ExactlyOnce 认领。已检查的启动依赖单独交付的 #1407 准入失败处理。

## 以新 generation 重试 Failed 的零工具 Child

一个终态为 Failed 的零工具 Child，若没有已提交的 Assistant/Tool、native transcript、旧的自有认领、游标或私有 checkpoint 标记，就可以通过正常的 `SubAgent.send_message(auto_run=true)` 接受一条新的有界输入。其实际 Inbox 必须恰好包含一个合格的新 generation，且晚于失败的那次激活。既有的 Directory 认领/启动会在同一逻辑 Child/birth 上创建新的 attempt 与租约 owner；这不是旧激活的续租。Live、已过期的 Live、Succeeded 以及歧义的旧状态一律拒绝。只读 Glob 绝不进入这条重试路径；它保留自己的仅新鲜准入。

在发送重试 Run 之前，Host 会认领那条实际输入，并调用与 Running 修正相同的 `ActorInputCheckpoint` 消费方。它采纳已提交的 Session，发送输入前前缀加上既有的类型化投递，并在姿态不变与当前 Host 策略检查之下，把其权限审计见证绑定到那次实际回读。它要求在自有 ACK 之前获得精确的已检查启动确认。被拒绝或未确认的 checkpoint 不派发 Run/provider，也不做 ACK 或普通保存。成功的普通回复以新围栏追加；被拒绝的 worker 推理/缓存内容绝不会成为 Host 规范历史。

全新的 required-context bus worker 会获得唯一的物理 mailbox ID。被杀掉 worker 的未 ACK Run 会留在其旧 mailbox 中，而不是由重试进程重放。逻辑 Child 身份/birth、缓存命名空间、类型化输入目标与权限保持不变；旧式的池化/直连/远程路由也保持不变。

这里只启用一次重试 worker Run；额外的待处理输入保持持久，但不受支持。同 generation 的 checkpoint/旧认领恢复是独立事项：其实际的服务器 Run 调用方必须先省去旧式的全量保存/重置，然后以当前已过期的物理替换认领复用该输入消费方，并要求精确的 `AlreadyCheckpointed`/启动/ACK。当前的 birth/血统、前缀与租约检查仍然必要；该路径不主张租约续期、自动重启、worker 释放、远程、重新指派或 ExactlyOnce。
