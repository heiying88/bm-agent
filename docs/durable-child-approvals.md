# 持久化的子代审批注册表

来自 actor 子代的人工审批会在暴露给客户端之前，持久化到 `<data-dir>/approvals/child-approvals-v1.json`。带版本的注册表记录单调的状态转换：

```text
pending -> decision_recorded -> delivered | delivery_failed
pending -> expired | delivery_failed
```

临时文件刷新之后，该文件会被原子替换。上一份完整文件会作为最后已知良好（last-known-good）备份保留。不支持的 schema 或两份副本同时损坏，都会让服务端启动按失败关闭处理，而不是悄悄丢弃审批。

审批决定以 `(child_session_id, request_id)` 为键，一次性生效。决定会在投递给活跃 worker 之前提交，因此重复与并发的响应不会执行两次。服务端启动时不存在旧 actor 传输的幸存证明；因此任何持久化的 pending 或 decision-recorded 请求都会以原因 `server_restart` 对账为 `delivery_failed`，并写入持久的账户变更流。权威的待处理快照与浏览器重载水合（hydration）仍然是 #592 的第二阶段 PR B。
