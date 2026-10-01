# 编辑演示文稿

## 基于模板的工作流

将现有演示文稿用作模板时：

1. **分析现有幻灯片**：
   ```bash
   python scripts/thumbnail.py template.pptx
   python -m markitdown template.pptx
   ```
   查看 `thumbnails.jpg` 了解版式，查看 markitdown 输出了解占位符文本。

2. **规划幻灯片映射**：为每个内容小节挑选一页模板幻灯片。

   ⚠️ **版式要多样化** —— 版式单调是常见的失败模式。不要默认套用基础的“标题 + 项目符号”页。主动挖掘：
   - 多栏版式（双栏、三栏）
   - 图文组合
   - 满版图片叠加文字
   - 引言页或重点标注页
   - 小节分隔页
   - 数据/数字标注页
   - 图标网格或“图标 + 文字”行

   **避免**：每一页都重复使用同一种文字密集的版式。

   让内容类型与版式风格匹配（例如：要点 → 项目符号页，团队信息 → 多栏页，客户评价 → 引言页）。

3. **解包**：`python scripts/office/unpack.py template.pptx unpacked/`

4. **搭建演示文稿结构**（自己动手做，不要交给子代理）：
   - 删除不需要的幻灯片（从 `<p:sldIdLst>` 中移除）
   - 复制需要复用的幻灯片（`add_slide.py`）
   - 在 `<p:sldIdLst>` 中调整幻灯片顺序
   - **在第 5 步之前完成全部结构性改动**

5. **编辑内容**：更新每个 `slide{N}.xml` 中的文本。
   **此处如有子代理可用就使用** —— 每页幻灯片都是独立的 XML 文件，子代理可以并行编辑。

6. **清理**：`python scripts/clean.py unpacked/`

7. **打包**：`python scripts/office/pack.py unpacked/ output.pptx --original template.pptx`

---

## 脚本

| 脚本 | 用途 |
|--------|---------|
| `unpack.py` | 解压 PPTX 并格式化输出 |
| `add_slide.py` | 复制幻灯片或从版式创建 |
| `clean.py` | 移除孤立文件 |
| `pack.py` | 校验后重新打包 |
| `thumbnail.py` | 生成幻灯片视觉网格图 |

### unpack.py

```bash
python scripts/office/unpack.py input.pptx unpacked/
```

解压 PPTX，格式化输出 XML，转义智能引号。

### add_slide.py

```bash
python scripts/add_slide.py unpacked/ slide2.xml      # 复制幻灯片
python scripts/add_slide.py unpacked/ slideLayout2.xml # 从版式创建
```

会打印需要加入 `<p:sldIdLst>` 指定位置的 `<p:sldId>`。

### clean.py

```bash
python scripts/clean.py unpacked/
```

移除不在 `<p:sldIdLst>` 中的幻灯片、未被引用的媒体文件以及孤立的关系文件。

### pack.py

```bash
python scripts/office/pack.py unpacked/ output.pptx --original input.pptx
```

校验、修复、压缩 XML，并重新编码智能引号。

### thumbnail.py

```bash
python scripts/thumbnail.py input.pptx [output_prefix] [--cols N]
```

生成以幻灯片文件名为标签的 `thumbnails.jpg`。默认 3 列，每张网格图最多 12 页。

**仅用于模板分析**（挑选版式）。视觉质量检查请用 `soffice` + `pdftoppm` 生成全尺寸的单页幻灯片图片——见 SKILL.md。

---

## 幻灯片操作

幻灯片顺序记录在 `ppt/presentation.xml` 的 `<p:sldIdLst>` 中。

**调整顺序**：重新排列 `<p:sldId>` 元素。

**删除**：移除 `<p:sldId>`，然后运行 `clean.py`。

**新增**：使用 `add_slide.py`。绝不要手动复制幻灯片文件——脚本会处理备注引用、Content_Types.xml 和关系 ID，这些恰恰是手动复制容易遗漏的。

---

## 编辑内容

**子代理**：如有可用，在此处使用（完成第 4 步之后）。每页幻灯片都是独立的 XML 文件，子代理可以并行编辑。给子代理的提示中应包含：
- 要编辑的幻灯片文件路径
- **“所有修改一律使用 Edit 工具”**
- 下文的格式规则和常见陷阱

对每一页幻灯片：
1. 读取该幻灯片的 XML
2. 找出全部占位符内容——文本、图片、图表、图标、说明文字
3. 将每个占位符替换为最终内容

**使用 Edit 工具，不要用 sed 或 Python 脚本。**Edit 工具会强制你明确改什么、在哪里改，因此更可靠。

### 格式规则

- **所有标题、小标题和行内标签一律加粗**：在 `<a:rPr>` 上使用 `b="1"`。包括：
  - 幻灯片标题
  - 幻灯片内的小节标题
  - 行首的行内标签（如 "Status:"、"Description:"）
- **绝不使用 Unicode 项目符号（•）**：用 `<a:buChar>` 或 `<a:buAutoNum>` 做正规的列表格式
- **项目符号保持一致**：让项目符号从版式继承。只指定 `<a:buChar>` 或 `<a:buNone>`。

---

## 常见陷阱

### 模板适配

当源内容的条目少于模板时：
- **把多余元素整体删除**（图片、形状、文本框），而不是只清空文字
- 清空文字内容后检查是否留下孤立的视觉元素
- 运行视觉质量检查，发现数量不匹配的问题

用长度不同的内容替换文本时：
- **更短的替换**：通常安全
- **更长的替换**：可能溢出或意外换行
- 文字修改后用视觉质量检查验证
- 考虑截断或拆分内容，以符合模板的设计约束

**模板槽位 ≠ 源条目**：如果模板有 4 名团队成员而源数据只有 3 人，应删除第 4 名成员的整个分组（图片 + 文本框），而不是只删文字。

### 多条目内容

如果源内容有多个条目（编号列表、多个小节），为每个条目创建独立的 `<a:p>` 元素——**绝不拼接成一个字符串**。

**❌ 错误** —— 所有条目挤在一个段落里：
```xml
<a:p>
  <a:r><a:rPr .../><a:t>Step 1: Do the first thing. Step 2: Do the second thing.</a:t></a:r>
</a:p>
```

**✅ 正确** —— 独立段落配加粗标题：
```xml
<a:p>
  <a:pPr algn="l"><a:lnSpc><a:spcPts val="3919"/></a:lnSpc></a:pPr>
  <a:r><a:rPr lang="en-US" sz="2799" b="1" .../><a:t>Step 1</a:t></a:r>
</a:p>
<a:p>
  <a:pPr algn="l"><a:lnSpc><a:spcPts val="3919"/></a:lnSpc></a:pPr>
  <a:r><a:rPr lang="en-US" sz="2799" .../><a:t>Do the first thing.</a:t></a:r>
</a:p>
<a:p>
  <a:pPr algn="l"><a:lnSpc><a:spcPts val="3919"/></a:lnSpc></a:pPr>
  <a:r><a:rPr lang="en-US" sz="2799" b="1" .../><a:t>Step 2</a:t></a:r>
</a:p>
<!-- 继续此模式 -->
```

从原段落复制 `<a:pPr>` 以保留行距。标题使用 `b="1"`。

### 智能引号

unpack/pack 会自动处理。但 Edit 工具会把智能引号转换成 ASCII 字符。

**新增带引号的文本时，使用 XML 实体：**

```xml
<a:t>the &#x201C;Agreement&#x201D;</a:t>
```

| 字符 | 名称 | Unicode | XML 实体 |
|-----------|------|---------|------------|
| `“` | 左双引号 | U+201C | `&#x201C;` |
| `”` | 右双引号 | U+201D | `&#x201D;` |
| `‘` | 左单引号 | U+2018 | `&#x2018;` |
| `’` | 右单引号 | U+2019 | `&#x2019;` |

### 其他

- **空白字符**：对首尾带空格的 `<a:t>` 使用 `xml:space="preserve"`
- **XML 解析**：使用 `defusedxml.minidom`，不要用 `xml.etree.ElementTree`（会破坏命名空间）
