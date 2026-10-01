# 显式的 required-packet Child 报告

`SubAgent.get` 为一次性 required-packet Child 提供两个 opt-in 视图：

```json
{"action":"get","child_session_id":"child-id","view":"result_binding"}
```

经直系父级授权后，该视图返回真实 durable Child 的 `child_created_at` 和 `assignment_sha256`。required-packet 创建也会在其既有的 `context_packet` 对象中返回这两项。请提供这些确切的选择器：

```json
{"action":"get","child_session_id":"child-id","view":"typed_result","expected_child_created_at":"2026-09-26T00:00:00Z","expected_assignment_sha256":"<64 lowercase hex>"}
```

只有受信任的工具上下文会提供父级身份。新视图拒绝分页、消息选择器和未知参数。旧视图拒绝这两个新选择器；除此之外，其既有的分页和嵌套直系父级行为保持不变。

在绑定之前，要求在原始 packet 或 assignment 中使用以下报告格式：

```json
{"version":1,"outcome":"blocked","summary":"Need a decision","reported_evidence":[{"description":"Source inspected","reference":"src/example.rs","sha256":null}],"reported_verification":[{"check":"test","reported_status":"not_run","details":""}],"proposals":[],"blockers":["Choose an approach"],"open_decisions":[]}
```

整个 Assistant 内容必须是单个 JSON 对象。所有成员均为必填，包括显式的 null；重复成员、未知成员、纯文本/代码围栏/尾随值、错误类型和不支持的枚举都会被拒绝。outcome 取值为 completed、partial、blocked 或 failed。校验状态为 passed、failed、not_run 或 unknown。summary 非空且最多 2048 UTF-8 字节。evidence/verification 以及每个字符串数组最多 16 个条目。evidence 的 description/reference、verification 的 details 和数组条目最多 1024 字节；check 最多 256 字节。可为 null 的引用在存在时必须非空；可为 null 的 SHA256 是 64 个字符的小写十六进制。引用是不透明的上报声明：Inspect 不会对其路径或 URL 执行任何 I/O，也绝不执行 proposal。

成功响应将 `child_report` 与带 `kind=durable_snapshot` 的 `host_observation` 分开返回。Child 上报的 blocked 可以与 Host 已完成的状态并存。Inspect 会检查当前绑定、父级出生/血缘/Project 以及每个已记录的源 ID/内容，包括被省略的可选源。它只选择最新的 Assistant，要求满足既有的 completed/after-last-User 谓词，并拒绝非 plain、Commentary、工具调用或压缩的替换内容。它绝不回退到更早的可解析报告。

父/子的完整 Store 读取是彼此独立的快照。这既不是原子实时权威，也不是回调/运行归属、已验证的证据或新的权限授予。Inspect 不会 provision、重试或采纳任何 worker。旧版/未绑定和常驻状态不受支持；传播上来的 Store 解码失败按普通工具错误处理。读取保留既有的全 Store 语义：普通 Child 的 runtime sidecar 损坏时仍可回退到其有效的 Main 快照。本切片不增加更严格的原始文件读取器，也不修复这一相邻回退。

预期的内容失败只返回 view/version/available=false/reason：`typed_result_unsupported`、`result_binding_invalid`、`stale_result_selector`、`stale_parent_context`、`report_absent`、`report_not_current_final`、`report_malformed` 或 `result_budget_exceeded`。过期选择器绝不会自动返回替换选择器。显式发现是另一个独立决策。

原始 Assistant 解析上限为 8192 字节。转义后的紧凑负载和实际序列化的 ToolResult（包括其 result String 的第二层转义和显示字段）都必须控制在 8192 字节内。溢出时返回小型不可用信封，不截断声明、不丢弃出处。对于更大或格式错误的纯文本报告，请使用未变更的旧版结果/消息分页。

验收夹具扩展真实编译的原生 serve/current_exe worker 路径，然后停止并重开其 Store 以进行实际的 SubAgent 工具检查。只有远程模型是伪造的。宿主测试线程的显式栈大小仅用于夹具；生产原生宿主和 worker 保留默认栈设置。源码夹具和静态断言不是测试执行回执。
