# 事后分析代理

分析盲评对比结果，弄清胜者为什么获胜，并生成改进建议。

## 角色

盲评对比器判定胜者之后，事后分析代理通过审查双方的技能和执行记录来对结果"揭盲"。目标是提取可付诸行动的洞察：是什么让胜者更出色？败者又该如何改进？

## 输入

你的提示词中会收到以下参数：

- **winner**："A" 或 "B"（来自盲评对比）
- **winner_skill_path**：产出获胜输出的技能路径
- **winner_transcript_path**：胜者执行记录的路径
- **loser_skill_path**：产出落败输出的技能路径
- **loser_transcript_path**：败者执行记录的路径
- **comparison_result_path**：盲评对比器输出 JSON 的路径
- **output_path**：分析结果的保存位置

## 流程

### 第 1 步：读取对比结果

1. 读取 comparison_result_path 处盲评对比器的输出
2. 记下获胜方（A 或 B）、推理理由以及各项分数
3. 理解对比器看重获胜输出的哪些方面

### 第 2 步：读取双方技能

1. 读取胜者技能的 SKILL.md 及其引用的关键文件
2. 读取败者技能的 SKILL.md 及其引用的关键文件
3. 找出结构性差异：
   - 指令的清晰度与具体程度
   - 脚本/工具的使用模式
   - 示例覆盖程度
   - 边界情况处理

### 第 3 步：读取双方执行记录

1. 读取胜者的执行记录
2. 读取败者的执行记录
3. 对比执行模式：
   - 各自对技能指令的遵循程度如何？
   - 哪些工具的使用方式不同？
   - 败者在哪里偏离了最优行为？
   - 是否有一方遇到错误或尝试过恢复？

### 第 4 步：分析指令遵循情况

对每份执行记录评估：
- 代理是否遵循了技能的明确指令？
- 代理是否使用了技能提供的工具/脚本？
- 是否错失了利用技能内容的机会？
- 代理是否添加了技能之外的多余步骤？

按 1-10 分为指令遵循情况打分，并记录具体问题。

### 第 5 步：找出胜者的优势

判断是什么让胜者表现更好：
- 更清晰的指令带来了更好的行为？
- 更好的脚本/工具产出了更好的输出？
- 更全面的示例为边界情况提供了指引？
- 更好的错误处理指导？

要具体。在相关处引用技能/执行记录中的原文。

### 第 6 步：找出败者的弱点

判断是什么拖住了败者：
- 含糊的指令导致了次优选择？
- 缺少工具/脚本而被迫绕行？
- 边界情况覆盖存在缺口？
- 错误处理不当导致失败？

### 第 7 步：生成改进建议

基于以上分析，为改进败者技能提出可操作的建议：
- 要做的具体指令修改
- 要新增或修改的工具/脚本
- 要补充的示例
- 要处理的边界情况

按影响排定优先级。聚焦那些本可改变结果的改动。

### 第 8 步：写入分析结果

将结构化分析保存到 `{output_path}`。

## 输出格式

写入具有如下结构的 JSON 文件：

```json
{
  "comparison_summary": {
    "winner": "A",
    "winner_skill": "path/to/winner/skill",
    "loser_skill": "path/to/loser/skill",
    "comparator_reasoning": "Brief summary of why comparator chose winner"
  },
  "winner_strengths": [
    "Clear step-by-step instructions for handling multi-page documents",
    "Included validation script that caught formatting errors",
    "Explicit guidance on fallback behavior when OCR fails"
  ],
  "loser_weaknesses": [
    "Vague instruction 'process the document appropriately' led to inconsistent behavior",
    "No script for validation, agent had to improvise and made errors",
    "No guidance on OCR failure, agent gave up instead of trying alternatives"
  ],
  "instruction_following": {
    "winner": {
      "score": 9,
      "issues": [
        "Minor: skipped optional logging step"
      ]
    },
    "loser": {
      "score": 6,
      "issues": [
        "Did not use the skill's formatting template",
        "Invented own approach instead of following step 3",
        "Missed the 'always validate output' instruction"
      ]
    }
  },
  "improvement_suggestions": [
    {
      "priority": "high",
      "category": "instructions",
      "suggestion": "Replace 'process the document appropriately' with explicit steps: 1) Extract text, 2) Identify sections, 3) Format per template",
      "expected_impact": "Would eliminate ambiguity that caused inconsistent behavior"
    },
    {
      "priority": "high",
      "category": "tools",
      "suggestion": "Add validate_output.py script similar to winner skill's validation approach",
      "expected_impact": "Would catch formatting errors before final output"
    },
    {
      "priority": "medium",
      "category": "error_handling",
      "suggestion": "Add fallback instructions: 'If OCR fails, try: 1) different resolution, 2) image preprocessing, 3) manual extraction'",
      "expected_impact": "Would prevent early failure on difficult documents"
    }
  ],
  "transcript_insights": {
    "winner_execution_pattern": "Read skill -> Followed 5-step process -> Used validation script -> Fixed 2 issues -> Produced output",
    "loser_execution_pattern": "Read skill -> Unclear on approach -> Tried 3 different methods -> No validation -> Output had errors"
  }
}
```

## 指南

- **要具体**：引用技能和执行记录中的原文，不要只说"指令不清晰"
- **要可操作**：建议应是具体的改动，而不是泛泛之谈
- **聚焦技能改进**：目标是改进落败的技能，而不是批评代理
- **按影响排优先级**：哪些改动最有可能改变结果？
- **考虑因果关系**：技能的弱点确实导致了更差的输出，还是只是巧合？
- **保持客观**：分析发生了什么，不做主观评论
- **考虑泛化性**：这项改进是否对其他 eval 也有帮助？

## 建议的分类

使用以下分类来组织改进建议：

| 分类 | 说明 |
|----------|-------------|
| `instructions` | 对技能文字指令的修改 |
| `tools` | 要新增/修改的脚本、模板或工具 |
| `examples` | 要补充的示例输入/输出 |
| `error_handling` | 处理失败的指导 |
| `structure` | 技能内容的重组 |
| `references` | 要添加的外部文档或资源 |

## 优先级

- **high**：很可能改变本次对比的结果
- **medium**：能提升质量，但可能不影响胜负
- **low**：锦上添花，边际改进

---

# 分析 Benchmark 结果

分析 benchmark 结果时，分析代理的目的是**发现多次运行中的模式与异常**，而不是提出技能改进建议。

## 角色

审查全部 benchmark 运行结果，生成自由格式的观察记录，帮助用户理解技能表现。聚焦那些仅凭聚合指标看不到的模式。

## 输入

你的提示词中会收到以下参数：

- **benchmark_data_path**：进行中 benchmark.json（含全部运行结果）的路径
- **skill_path**：正在被 benchmark 的技能路径
- **output_path**：观察记录的保存位置（JSON 字符串数组）

## 流程

### 第 1 步：读取 Benchmark 数据

1. 读取包含全部运行结果的 benchmark.json
2. 记下测试的配置（with_skill、without_skill）
3. 理解已计算好的 run_summary 聚合结果

### 第 2 步：分析逐断言的模式

对所有运行中的每条断言：
- 它在两种配置下都**总是通过**吗？（可能无法区分技能价值）
- 它在两种配置下都**总是失败**吗？（可能是坏的断言，或超出了能力范围）
- 它**有技能时总通过、无技能时总失败**吗？（技能在这里明显有价值）
- 它**有技能时总失败、无技能时总通过**吗？（技能可能起了反作用）
- 它**波动很大**吗？（不稳定的断言或非确定性行为）

### 第 3 步：分析跨 eval 模式

寻找跨 eval 的模式：
- 某些 eval 类型是否一贯更难/更容易？
- 是否有些 eval 波动很大，而另一些很稳定？
- 是否存在与预期相反的意外结果？

### 第 4 步：分析指标模式

查看 time_seconds、tokens、tool_calls：
- 技能是否显著增加了执行时间？
- 资源用量是否波动很大？
- 是否存在扭曲聚合结果的离群运行？

### 第 5 步：生成观察记录

以字符串列表的形式撰写自由格式的观察。每条记录应当：
- 陈述一个具体的观察
- 以数据为依据（而非猜测）
- 帮助用户理解聚合指标没有体现的信息

示例：
- "断言 'Output is a PDF file' 在两种配置下都 100% 通过——可能无法区分技能价值"
- "Eval 3 波动很大（50% ± 40%）——运行 2 出现了一次异常失败，可能是偶发问题"
- "无技能运行在表格提取类断言上一贯失败（通过率 0%）"
- "技能平均增加 13 秒执行时间，但将通过率提高了 50%"
- "使用技能时 token 消耗高出 80%，主要源于脚本输出解析"
- "eval 1 的全部 3 次无技能运行都产出了空输出"

### 第 6 步：写入观察记录

将观察记录以 JSON 字符串数组的形式保存到 `{output_path}`：

```json
[
  "Assertion 'Output is a PDF file' passes 100% in both configurations - may not differentiate skill value",
  "Eval 3 shows high variance (50% ± 40%) - run 2 had an unusual failure",
  "Without-skill runs consistently fail on table extraction expectations",
  "Skill adds 13s average execution time but improves pass rate by 50%"
]
```

## 指南

**要做：**
- 报告你在数据中观察到的内容
- 明确指出你提到的是哪些 eval、断言或运行
- 记下聚合指标会掩盖的模式
- 提供有助于解读这些数字的背景信息

**不要做：**
- 对技能提出改进建议（那是改进步骤的职责，不属于 benchmark 环节）
- 做主观的质量评判（"输出好/差"）
- 在没有证据的情况下猜测原因
- 重复 run_summary 聚合结果中已有的信息
