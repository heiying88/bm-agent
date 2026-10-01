# opt-in 检查点的当前祖先校验

transcript 追加端口和自有输入检查点端口现在复用既有模型上下文检查点的当前祖先观察。这修复了旧端口；它不启用任何 provider、worker、Inbox 释放或 ACK 调用方。

在既有的共享 lifecycle → 共享 Task → 精确目标 Session 守卫之下，一个自有文件系统作业捕获祖先 Main、Runtime、Root 工具证明和 Supervisor 证明的存在性及完整字节。既有的纯 ActorDirectory 血缘读取器独立校验规范的 parent/root/depth/birth/Project。记录在案的祖先 ID/出生必须匹配，元数据不能回滚，修订间隔达到两次及以上即拒绝。提取出的私有见证和比较逻辑保持模型上下文端口的行为。

最终启动的作业在写入之前以及真实的 BeforeReplace 屏障处，再次检查目标和祖先观察。自有输入的 Already/恢复在其既有的物理 Inbox claim 持有者之下执行相同的当前检查；中间 Child 被删除或祖先变更时直接拒绝，不发布检查点也不发送 ACK。transcript 没有 Already/重放通道。替换之后出错仍为 OutcomeUnconfirmed，重试之前需要完整重载。

不引入祖先锁、初始化/修复、新 journal、持久化格式、权限授予或 API。完整启动的作业在返回/错误清理和调用方取消的全过程中持有相同的实际守卫 Arc。既有的整文件读取和协作写入者边界保持不变；本切片不添加有界历史读取、no-follow/retained-FD 安全或任意外部写入保护。

真实的文件系统夹具覆盖：合法的嵌套成功、叶子仍在时合法删除中间 Child、Project 不匹配、自洽却损坏的出生/元数据、修订间隔、BeforeReplace 期间一次合法的独立父级保存、全部四个祖先见证文件，以及输入替换未确认之后的冷重载。既有的中止/关闭守卫测试仍是所有权的验收边界。
