# Bamboo Edit 模糊匹配设计

## 背景

Bamboo 的 `Edit` 目前依赖带行尾归一化（`LF` / `CRLF`）的精确子串匹配。这安全且可预测，但给 LLM 驱动的编辑带来了一个现实失效模式：

- 模型复现目标块时常常带轻微的空白漂移
- 精确匹配随之以 `not found` 失败
- 模型可能以缩短 `SEARCH` 块或改用 `replace_all` 来应对
- 这种回退行为加大了改动范围超出预期的风险

我们已收紧 `replace_all` 与大范围防护。下一步是在不削弱安全性的前提下改进匹配易用性。

---

## 目标

1. 减少仅由无害空白变化引起的 `not found` 失败。
2. 保留 Bamboo 现有的安全模型：
   - 先读后改（read-before-edit）
   - 默认拒绝歧义
   - touched-lines 范围限制
   - 较大改动优先走 patch 模式
3. 保持匹配行为可解释、可测试。
4. 避免把一次精确编辑悄悄变成大范围结构性重写。

---

## 非目标

1. 第一轮迭代不支持语义级 AST 重写。
2. 不对任意无关文本块做模糊匹配。
3. 不在多个弱匹配之间自动挑选。
4. 第一阶段不把模糊匹配用于 `replace_all`。

---

## 现状

`Edit` 目前做的事：

- legacy 模式下精确搜索（`old_string` / `new_string`）
- patch 模式下精确匹配 `SEARCH` 块
- `LF` / `CRLF` 归一化变体
- 借助 `line_number` 的重复检测
- 歧义即拒绝

实现主要位于：

- `crates/bamboo-tools/src/tools/edit.rs`
- `crates/bamboo-tools/src/tools/file_change.rs`

---

## 设计原则

### 1. 精确优先，模糊其次

匹配顺序应为：

1. 精确匹配
2. 归一化精确匹配（已有）
3. 空白感知的模糊匹配
4. 否则失败

这让所有现有精确匹配的行为保持不变。

### 2. 模糊匹配仍必须唯一

仅当以下条件全部满足，模糊匹配才可接受：

- 恰好只有一个足够强的候选，且
- 任何次优候选都明显更差，且
- touched-lines 范围仍通过现有安全护栏

若多个候选同样好，返回歧义错误。

### 3. 模糊匹配只应宽容格式漂移

第一阶段模糊匹配应容忍：

- 缩进宽度差异
- 行尾空白差异
- 狭义上的空行归一化
- LF/CRLF 差异

不应容忍：

- 行序重排
- 非空白 token 的插入或删除
- 标识符或标点漂移
- 跨相距很远区域的匹配

---

## 拟议的三阶段推进

## 阶段 1：空白归一化的块匹配

### 范围

先只应用于 patch 模式。

### 行为

当 `SEARCH` 块精确匹配失败时：

1. 把 `SEARCH` 与候选窗口都切成行
2. 逐行归一化：
   - 去掉行尾空白
   - 把 tab 转成 canonical 表示，或保留 tab 但单独比较缩进宽度
3. 去掉公共缩进偏移后再逐行比较
4. 要求逐行的非空白 token 内容完全一致

### 候选生成

不在文件里扫描每个可能的字节偏移，而是按行跨度生成候选窗口：

- 若 `SEARCH` 块有 `n` 行，则与连续 `n` 行的窗口比较
- 可选地也比较 `n +/- 1`——仅在启用空行归一化时；阶段 1 默认不启用

### 接受规则

仅当恰好一个窗口满足以下条件才接受：

- 行数相同
- 每行非空白 token 内容相同
- 允许缩进差异
- 无 token 变化

### 为什么先只做 patch 模式

patch 模式本就鼓励更丰富的上下文，是引入模糊行为更安全的位置。

---

## 阶段 2：legacy 模式模糊回退

把更窄版本的模糊匹配应用于 legacy 模式，但仅当：

- `replace_all == false`
- `line_number` 缺失，或指向单一候选附近
- `old_string` 跨多行，或足够特异

### 附加护栏

legacy 模式下不使用模糊匹配的情形：

- `old_string` 是单条短行
- `old_string` 少于可配置的 token 阈值
- 最佳匹配会触及大块 diff 区域

这样避免了去模糊匹配 `}` 或 `foo` 这类细小片段。

---

## 阶段 3：结构化匹配（可选，延后）

面向特定语言的可选后续工作：

- Rust：使用解析器感知的块边界
- TS/JS：使用轻量 AST 节点锚定
- JSON/YAML/TOML：键路径感知的编辑

这大概率应与通用 `Edit` 算法分开，可能做成若干专用 helper，而不是一层万能的模糊层。

---

## 匹配算法建议

## 阶段 1 算法：归一化行指纹匹配

对每一行：

- 保留原始行文本，用作替换边界
- 计算比较指纹：
  - 去掉行尾空白
  - 把前导空白串转成 `INDENT(n)` 标记，或忽略精确宽度
  - 行内非空白字符保持精确

示例：

```text
"    let x = 1;   " -> fingerprint: "let x = 1;"
"\tlet x = 1;"      -> fingerprint: "let x = 1;"
```

对一个块：

- 为所有行计算指纹
- 要求指纹序列完全相等
- 阶段 1 可选地要求空行数量一致

### 优点

- 简单
- 确定性
- 错误信息里容易解释
- 相比编辑距离搜索，误报风险更低

### 为什么不先用 Levenshtein

纯编辑距离方案更难推理，也更容易被滥用：

- 多个弱相似块可能显得等价
- 标点/token 丢失仍可能得到足够高的分
- 阈值调参会变得脆弱

对 Bamboo 来说，保留 token 的空白归一化策略是更好的第一步。

---

## 拟议的内部 API 形态

在 `edit.rs` 内部引入一个匹配模式抽象，例如：

```rust
enum MatchStrategy {
    Exact,
    NormalizedWhitespace,
}
```

以及候选收集函数，例如：

```rust
fn collect_exact_candidates(...)
fn collect_whitespace_normalized_candidates(...)
```

再用一个函数编排：

```rust
fn collect_candidates(...) -> Vec<ReplacementCandidate>
```

其中精确收集器先运行，只有当精确收集结果为空时才运行模糊收集器。

### 重要

不要在无元数据的情况下把精确候选与模糊候选混进同一个不加区分的池子。加上来源标记，例如：

```rust
enum MatchKind {
    Exact,
    NormalizedWhitespace,
}
```

这带来：

- 更清晰的错误信息
- 后续遥测
- 诸如「仅 patch 模式允许模糊匹配」的 policy 决策

---

## 模糊匹配安全规则

1. **阶段 1、2 均不对 `replace_all` 做模糊匹配**。
2. **多于一个候选通过阈值时不做模糊匹配**。
3. **搜索文本极短时不做模糊匹配**。
4. **替换后始终应用现有 touched-lines 护栏**。
5. **错误信息必须说明是否尝试过模糊匹配**。

示例错误：

```text
SEARCH content not found exactly. A whitespace-normalized match was attempted but found 2 ambiguous candidates at lines 120 and 188. Add more context.
```

---

## 错误信息策略

我们应改进错误信息，让模型学会正确的重试行为。

### 好的重试引导

- 给 `SEARCH` 增加更多上下文行
- 优先 patch 模式而非 `replace_all`
- 仅在目标块明确时使用 `line_number`

### 避免

- 过于急切地建议 `replace_all=true`
- 不带上下文的含糊 `not found`

---

## 测试计划

## 单元测试

### 精确行为不变

- 精确单匹配仍可用
- 无 `line_number` 时重复精确匹配仍被拒绝
- `line_number` 仍能为精确重复消歧

### 空白归一化成功用例

- 缩进宽度不同、token 相同
- 前导缩进 tab 与空格的差异
- 行尾空白差异
- LF 与 CRLF（已有，应保持绿）

### 空白归一化拒绝用例

- 标识符名不同
- 标点不同
- 块中间缺一行
- 两个同样好的空白归一化候选
- 模糊匹配会超出 touched-lines 护栏

### legacy 模式限制

- 短单行搜索禁用 legacy 模糊
- `replace_all` 不使用模糊逻辑

## E2E 测试

- 带缩进漂移的 patch 请求成功
- 含两个空白等价重复块的 patch 请求返回歧义错误
- 短 `replace_all` 仍被拒绝
- 大范围模糊候选仍被 touched-lines 限制拒绝

---

## 遥测 / 可观测性（可选但推荐）

引入模糊匹配时返回额外的 payload 字段：

- `match_kind: exact | normalized_whitespace`
- `fuzzy_match_attempted: bool`
- `fuzzy_candidate_count: number`

这有助于评估：

- 模糊匹配的刚需频率
- 歧义是否常见
- 精确匹配是否仍占主导

---

## 迁移路径

### 第 1 步

当前这步已完成：

- touched-lines 按真实 diff 计量
- 更强的 replace_all 护栏
- legacy 兼容不再依赖 `estimated_touched_lines`

### 第 2 步

在内部 feature flag 或保守默认值之下，实现仅限 patch 模式的空白归一化匹配。

### 第 3 步

增加针对性测试并对比：

- 精确成功率
- not-found 率
- 歧义率
- 意外大改动率

### 第 4 步

仅当指标表现良好时，才考虑 legacy 模式模糊回退。

---

## 建议小结

推荐的下一步实现是：

1. 保持精确匹配为主行为
2. 新增**仅 patch 模式的空白归一化匹配**
3. 任何歧义都拒绝
4. 不为 `replace_all` 启用模糊
5. 保留现有 touched-lines 与先读后改安全护栏

这让 Bamboo 获得 Claude 式匹配的大部分实际 UX 收益，而不必承担一个通用模糊文本搜索引擎的全部风险。
