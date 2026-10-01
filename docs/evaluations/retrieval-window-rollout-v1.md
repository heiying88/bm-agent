# 检索窗口发布证据 v1

本报告由隐私安全的聚合 JSONL 确定性生成。它不包含提示词、答案、查询、消息、记忆、工具参数/结果、文件路径或 provider 负载。

## 证据标识

- Schema：`1`
- 语料：`retrieval-window-synthetic-v1`
- 模型：`deterministic-fixture-v1`
- Provider：`offline`
- 配置：`retrieval-window-opt-in-v1`、`summary-default-v1`
- 来源：合成聚合
- 配对场景数：`6`
- 聚合样本数：`12`
- 查询类别：`cjk_longer`、`cjk_two_character`、`latin`、`mixed`、`negative`、`path_punctuation`
- 证据类型：`historical_fact`、`identifier`、`prior_decision`、`tool_argument`、`tool_result`

## 确定性搜索/索引证据

这是来自 [Bamboo #1156](https://github.com/bigduu/Bamboo-agent/issues/1156) / PR #1157 的生产形态索引证据，不是模型/任务质量证据。

- 迁移消息数：`5,001`；schema-v5 重建：`42,936 us`。
- 实时数据库字节：`1,511,424 -> 2,772,992`；采样峰值：`4,779,272`。
- 5,000 条消息的 Latin 中缀热延迟 p50/p95/p99：`1,094 / 1,440 / 1,653 us`。
- 有界双字符字面量回退 p50/p95/p99：`4,342 / 4,699 / 4,972 us`。

## 配对聚合结果

| 指标 | summary | retrieval_window |
|---|---:|---:|
| 样本数 | 6 | 6 |
| 任务完成 | 3/6 (50.0%) | 6/6 (100.0%) |
| 精确恢复 | 3/6 (50.0%) | 6/6 (100.0%) |
| Provider/模型调用 | 16 | 22 |
| 工具轮数 | 8 | 16 |
| 检索调用 / 命中 | 0 / 0 | 12 / 5 |
| 检索命中率 | 不可用 | 41.7% |
| 周边读取调用 | 0 | 5 |
| 检索截断数 | 0 | 1 |
| 溢出信号 | 1 | 0 |
| 用户重述 | 3 | 0 |
| 提示输入 token | 105000 | 111100 |
| 输出 token | 5300 | 5860 |
| 缓存读取 token | 69000 | 71300 |
| 缓存创建 token | 17300 | 19000 |
| 缓存写入 token | 不可用 | 不可用 |
| 缓存占比 | 65.7% | 64.2% |
| 任务延迟 p50/p95/p99 ms | 1800 / 2800 / 2800 | 2000 / 3100 / 3100 |
| 检索延迟 p50/p95/p99 ms | 不可用 | 140 / 210 / 210 |
| 上下文纪元 / 重启 / 缓存边界 | 16 / 3 / 11 | 16 / 3 / 11 |

配对差值（`retrieval_window - summary`）：provider/模型调用 `+6`，工具轮数 `+8`。

## 现有 `session_history_current` 工具指标

实时 `metrics.db` 观测：不可用。本报告不把不可用的调用、成功或延迟当作零。运行时继续使用现有的 `tool_call_metrics` 写入器；未添加并行的 token 日志写入器。

## 默认策略决策

**决策：保持 `summary` 为默认值，`retrieval_window` 保持可选启用。**

已核验的证据是合成聚合覆盖率，并非有代表性的真实模型任务质量证据；它不足以支撑默认策略的晋升。
