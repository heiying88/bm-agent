---
name: docx
description: 当用户想要创建、读取、编辑或操作 Word 文档（.docx 文件）时使用本技能。触发场景包括：提到 “Word doc”、“word document”、“.docx”，或要求生成带目录、标题、页码、信头等格式的专业文档；从 .docx 文件中提取或重组内容；在文档中插入或替换图片；在 Word 文件中执行查找替换；处理修订或批注；或将内容转换成一份精美的 Word 文档。如果用户要求以 Word 或 .docx 文件形式交付“报告”、“备忘录”、“信函”、“模板”等产物，也使用本技能。不要用于 PDF、电子表格、Google Docs 或与文档生成无关的常规编程任务。
license: 专有许可证。完整条款见 LICENSE.txt
---

# DOCX 创建、编辑与分析

## 概述

.docx 文件是一个内含多个 XML 文件的 ZIP 压缩包。

## 速查表

| 任务 | 方法 |
|------|----------|
| 读取/分析内容 | 用 `pandoc` 或解包获取原始 XML |
| 新建文档 | 使用 `docx-js`——见下文"创建新文档" |
| 编辑现有文档 | 解包 → 编辑 XML → 重新打包——见下文"编辑现有文档" |

### 将 .doc 转换为 .docx

旧版 `.doc` 文件必须先转换才能编辑：

```bash
python scripts/office/soffice.py --headless --convert-to docx document.doc
```

### 读取内容

```bash
# 提取文本（含修订）
pandoc --track-changes=all document.docx -o output.md

# 直接访问原始 XML
python scripts/office/unpack.py document.docx unpacked/
```

### 转换为图片

```bash
python scripts/office/soffice.py --headless --convert-to pdf document.docx
pdftoppm -jpeg -r 150 document.pdf page
```

### 接受修订

要生成一份已接受全部修订的干净文档（需要 LibreOffice）：

```bash
python scripts/accept_changes.py input.docx output.docx
```

---

## 创建新文档

用 JavaScript 生成 .docx 文件，然后进行校验。安装：`npm install -g docx`

### 设置
```javascript
const { Document, Packer, Paragraph, TextRun, Table, TableRow, TableCell, ImageRun,
        Header, Footer, AlignmentType, PageOrientation, LevelFormat, ExternalHyperlink,
        InternalHyperlink, Bookmark, FootnoteReferenceRun, PositionalTab,
        PositionalTabAlignment, PositionalTabRelativeTo, PositionalTabLeader,
        TabStopType, TabStopPosition, Column, SectionType,
        TableOfContents, HeadingLevel, BorderStyle, WidthType, ShadingType,
        VerticalAlign, PageNumber, PageBreak } = require('docx');

const doc = new Document({ sections: [{ children: [/* 内容 */] }] });
Packer.toBuffer(doc).then(buffer => fs.writeFileSync("doc.docx", buffer));
```

### 校验
文件创建后要校验。如果校验失败，先解包、修复 XML，再重新打包。
```bash
python scripts/office/validate.py doc.docx
```

### 页面尺寸

```javascript
// 关键：docx-js 默认为 A4，而不是 US Letter
// 务必显式设置页面尺寸，以保证效果一致
sections: [{
  properties: {
    page: {
      size: {
        width: 12240,   // 8.5 英寸（DXA）
        height: 15840   // 11 英寸（DXA）
      },
      margin: { top: 1440, right: 1440, bottom: 1440, left: 1440 } // 1 英寸页边距
    }
  },
  children: [/* 内容 */]
}]
```

**常见纸张尺寸（DXA 单位，1440 DXA = 1 英寸）：**

| 纸张 | 宽 | 高 | 内容宽度（1 英寸页边距） |
|-------|-------|--------|---------------------------|
| US Letter | 12,240 | 15,840 | 9,360 |
| A4（默认） | 11,906 | 16,838 | 9,026 |

**横向排版：**docx-js 会在内部交换宽和高，因此请传入纵向尺寸，交换由它完成：
```javascript
size: {
  width: 12240,   // 短边作为 width 传入
  height: 15840,  // 长边作为 height 传入
  orientation: PageOrientation.LANDSCAPE  // docx-js 会在 XML 中交换两者
},
// 内容宽度 = 15840 - 左边距 - 右边距（取长边）
```

### 样式（覆盖内置标题样式）

使用 Arial 作为默认字体（普遍受支持）。标题保持黑色，便于阅读。

```javascript
const doc = new Document({
  styles: {
    default: { document: { run: { font: "Arial", size: 24 } } }, // 默认 12pt
    paragraphStyles: [
      // 重要：使用精确的 ID 才能覆盖内置样式
      { id: "Heading1", name: "Heading 1", basedOn: "Normal", next: "Normal", quickFormat: true,
        run: { size: 32, bold: true, font: "Arial" },
        paragraph: { spacing: { before: 240, after: 240 }, outlineLevel: 0 } }, // 目录需要 outlineLevel
      { id: "Heading2", name: "Heading 2", basedOn: "Normal", next: "Normal", quickFormat: true,
        run: { size: 28, bold: true, font: "Arial" },
        paragraph: { spacing: { before: 180, after: 180 }, outlineLevel: 1 } },
    ]
  },
  sections: [{
    children: [
      new Paragraph({ heading: HeadingLevel.HEADING_1, children: [new TextRun("Title")] }),
    ]
  }]
});
```

### 列表（绝不要使用 Unicode 项目符号）

```javascript
// ❌ 错误——绝不要手动插入项目符号字符
new Paragraph({ children: [new TextRun("• Item")] })  // 错误
new Paragraph({ children: [new TextRun("\u2022 Item")] })  // 错误

// ✅ 正确——通过 numbering 配置使用 LevelFormat.BULLET
const doc = new Document({
  numbering: {
    config: [
      { reference: "bullets",
        levels: [{ level: 0, format: LevelFormat.BULLET, text: "•", alignment: AlignmentType.LEFT,
          style: { paragraph: { indent: { left: 720, hanging: 360 } } } }] },
      { reference: "numbers",
        levels: [{ level: 0, format: LevelFormat.DECIMAL, text: "%1.", alignment: AlignmentType.LEFT,
          style: { paragraph: { indent: { left: 720, hanging: 360 } } } }] },
    ]
  },
  sections: [{
    children: [
      new Paragraph({ numbering: { reference: "bullets", level: 0 },
        children: [new TextRun("Bullet item")] }),
      new Paragraph({ numbering: { reference: "numbers", level: 0 },
        children: [new TextRun("Numbered item")] }),
    ]
  }]
});

// ⚠️ 每个 reference 都是独立的编号序列
// 相同 reference = 接续编号（1,2,3 之后是 4,5,6）
// 不同 reference = 重新编号（1,2,3 之后又是 1,2,3）
```

### 表格

**关键：表格需要双重宽度**——既要给表格设置 `columnWidths`，又要给每个单元格设置 `width`。缺一不可，否则表格在某些平台上会渲染错误。

```javascript
// 关键：务必设置表格宽度，保证渲染效果一致
// 关键：使用 ShadingType.CLEAR（而非 SOLID），防止出现黑色背景
const border = { style: BorderStyle.SINGLE, size: 1, color: "CCCCCC" };
const borders = { top: border, bottom: border, left: border, right: border };

new Table({
  width: { size: 9360, type: WidthType.DXA }, // 始终用 DXA（百分比在 Google Docs 中会出问题）
  columnWidths: [4680, 4680], // 各列之和必须等于表格宽度（DXA：1440 = 1 英寸）
  rows: [
    new TableRow({
      children: [
        new TableCell({
          borders,
          width: { size: 4680, type: WidthType.DXA }, // 每个单元格同样要设置
          shading: { fill: "D5E8F0", type: ShadingType.CLEAR }, // 用 CLEAR，不用 SOLID
          margins: { top: 80, bottom: 80, left: 120, right: 120 }, // 单元格内边距（向内占用，不会增加宽度）
          children: [new Paragraph({ children: [new TextRun("Cell")] })]
        })
      ]
    })
  ]
})
```

**表格宽度计算：**

始终使用 `WidthType.DXA`——`WidthType.PERCENTAGE` 在 Google Docs 中会出问题。

```javascript
// 表格宽度 = 各 columnWidth 之和 = 内容宽度
// US Letter、1 英寸页边距：12240 - 2880 = 9360 DXA
width: { size: 9360, type: WidthType.DXA },
columnWidths: [7000, 2360]  // 各列之和必须等于表格宽度
```

**宽度规则：**
- **始终使用 `WidthType.DXA`**——绝不要用 `WidthType.PERCENTAGE`（与 Google Docs 不兼容）
- 表格宽度必须等于各 `columnWidth` 之和
- 单元格 `width` 必须与对应的 `columnWidth` 一致
- 单元格 `margins` 是内部填充——它会缩减内容区域，而不是增加单元格宽度
- 通栏表格：使用内容宽度（页面宽度减去左右页边距）

### 图片

```javascript
// 关键：type 参数必填
new Paragraph({
  children: [new ImageRun({
    type: "png", // 必填：png、jpg、jpeg、gif、bmp、svg
    data: fs.readFileSync("image.png"),
    transformation: { width: 200, height: 150 },
    altText: { title: "Title", description: "Desc", name: "Name" } // 三项都必填
  })]
})
```

### 分页符

```javascript
// 关键：PageBreak 必须位于 Paragraph 内
new Paragraph({ children: [new PageBreak()] })

// 或者使用 pageBreakBefore
new Paragraph({ pageBreakBefore: true, children: [new TextRun("New page")] })
```

### 超链接

```javascript
// 外部链接
new Paragraph({
  children: [new ExternalHyperlink({
    children: [new TextRun({ text: "Click here", style: "Hyperlink" })],
    link: "https://example.com",
  })]
})

// 内部链接（书签 + 引用）
// 1. 在目标位置创建书签
new Paragraph({ heading: HeadingLevel.HEADING_1, children: [
  new Bookmark({ id: "chapter1", children: [new TextRun("Chapter 1")] }),
]})
// 2. 链接到该书签
new Paragraph({ children: [new InternalHyperlink({
  children: [new TextRun({ text: "See Chapter 1", style: "Hyperlink" })],
  anchor: "chapter1",
})]})
```

### 脚注

```javascript
const doc = new Document({
  footnotes: {
    1: { children: [new Paragraph("Source: Annual Report 2024")] },
    2: { children: [new Paragraph("See appendix for methodology")] },
  },
  sections: [{
    children: [new Paragraph({
      children: [
        new TextRun("Revenue grew 15%"),
        new FootnoteReferenceRun(1),
        new TextRun(" using adjusted metrics"),
        new FootnoteReferenceRun(2),
      ],
    })]
  }]
});
```

### 制表位

```javascript
// 让同一行文本右对齐（例如日期与标题分居两侧）
new Paragraph({
  children: [
    new TextRun("Company Name"),
    new TextRun("\tJanuary 2025"),
  ],
  tabStops: [{ type: TabStopType.RIGHT, position: TabStopPosition.MAX }],
})

// 点线前导符（例如目录样式）
new Paragraph({
  children: [
    new TextRun("Introduction"),
    new TextRun({ children: [
      new PositionalTab({
        alignment: PositionalTabAlignment.RIGHT,
        relativeTo: PositionalTabRelativeTo.MARGIN,
        leader: PositionalTabLeader.DOT,
      }),
      "3",
    ]}),
  ],
})
```

### 多栏布局

```javascript
// 等宽分栏
sections: [{
  properties: {
    column: {
      count: 2,          // 栏数
      space: 720,        // 栏间距（DXA），720 = 0.5 英寸
      equalWidth: true,
      separate: true,    // 栏间竖线
    },
  },
  children: [/* 内容在栏间自然流动 */]
}]

// 自定义栏宽（equalWidth 必须为 false）
sections: [{
  properties: {
    column: {
      equalWidth: false,
      children: [
        new Column({ width: 5400, space: 720 }),
        new Column({ width: 3240 }),
      ],
    },
  },
  children: [/* 内容 */]
}]
```

使用 `type: SectionType.NEXT_COLUMN` 新增一节，即可强制换栏。

### 目录

```javascript
// 关键：标题必须只使用 HeadingLevel——不要用自定义样式
new TableOfContents("Table of Contents", { hyperlink: true, headingStyleRange: "1-3" })
```

### 页眉/页脚

```javascript
sections: [{
  properties: {
    page: { margin: { top: 1440, right: 1440, bottom: 1440, left: 1440 } } // 1440 = 1 英寸
  },
  headers: {
    default: new Header({ children: [new Paragraph({ children: [new TextRun("Header")] })] })
  },
  footers: {
    default: new Footer({ children: [new Paragraph({
      children: [new TextRun("Page "), new TextRun({ children: [PageNumber.CURRENT] })]
    })] })
  },
  children: [/* 内容 */]
}]
```

### docx-js 关键规则

- **显式设置页面尺寸**——docx-js 默认为 A4；美国文档请使用 US Letter（12240 x 15840 DXA）
- **横向：传入纵向尺寸**——docx-js 会在内部交换宽和高；短边作为 `width`、长边作为 `height` 传入，并设置 `orientation: PageOrientation.LANDSCAPE`
- **绝不要使用 `\n`**——改用独立的 Paragraph 元素
- **绝不要使用 Unicode 项目符号**——通过 numbering 配置使用 `LevelFormat.BULLET`
- **PageBreak 必须位于 Paragraph 内**——单独使用会生成非法 XML
- **ImageRun 必须指定 `type`**——始终写明 png/jpg 等
- **表格 `width` 始终用 DXA**——绝不要用 `WidthType.PERCENTAGE`（在 Google Docs 中会出问题）
- **表格需要双重宽度**——`columnWidths` 数组和单元格 `width` 两者都要设置且互相匹配
- **表格宽度 = 各 columnWidth 之和**——使用 DXA 时务必让它们精确相加相等
- **始终设置单元格边距**——使用 `margins: { top: 80, bottom: 80, left: 120, right: 120 }` 以获得可读的内边距
- **使用 `ShadingType.CLEAR`**——表格底纹绝不要用 SOLID
- **绝不要用表格充当分隔线**——单元格有最小高度，会渲染成空框（页眉/页脚中同样如此）；应改在 Paragraph 上使用 `border: { bottom: { style: BorderStyle.SINGLE, size: 6, color: "2E75B6", space: 1 } }`。双栏页脚请使用制表位（见"制表位"一节），不要用表格
- **目录只认 HeadingLevel**——标题段落上不要用自定义样式
- **覆盖内置样式**——使用精确的 ID："Heading1"、"Heading2" 等
- **包含 `outlineLevel`**——目录必需（H1 为 0，H2 为 1，依此类推）

---

## 编辑现有文档

**请按顺序执行以下全部 3 个步骤。**

### 步骤 1：解包
```bash
python scripts/office/unpack.py document.docx unpacked/
```
该脚本会解出 XML、美化格式、合并相邻 run，并把智能引号转换为 XML 实体（`&#x201C;` 等），确保它们在编辑过程中不被破坏。使用 `--merge-runs false` 可跳过 run 合并。

### 步骤 2：编辑 XML

编辑 `unpacked/word/` 中的文件。常见模式见下文"XML 参考"。

修订和批注**统一用 "Claude" 作为作者**，除非用户明确要求使用其他名字。

**直接使用 Edit 工具做字符串替换，不要编写 Python 脚本。**脚本会引入不必要的复杂性。Edit 工具能清楚展示将要替换的内容。

**关键：新内容使用智能引号。**添加带撇号或引号的文本时，使用 XML 实体来生成智能引号：
```xml
<!-- 使用这些实体以获得专业排版 -->
<w:t>Here&#x2019;s a quote: &#x201C;Hello&#x201D;</w:t>
```
| 实体 | 字符 |
|--------|-----------|
| `&#x2018;` | ‘（左单引号） |
| `&#x2019;` | ’（右单引号/撇号） |
| `&#x201C;` | “（左双引号） |
| `&#x201D;` | ”（右双引号） |

**添加批注：**使用 `comment.py` 处理散落在多个 XML 文件中的样板内容（文本必须预先做 XML 转义）：
```bash
python scripts/comment.py unpacked/ 0 "Comment text with &amp; and &#x2019;"
python scripts/comment.py unpacked/ 1 "Reply text" --parent 0  # 回复批注 0
python scripts/comment.py unpacked/ 0 "Text" --author "Custom Author"  # 自定义作者名
```
然后向 document.xml 添加标记（见"XML 参考"中的"批注"）。

### 步骤 3：打包
```bash
python scripts/office/pack.py unpacked/ output.docx --original document.docx
```
会进行校验（含自动修复）、压缩 XML 并生成 DOCX。使用 `--validate false` 可跳过校验。

**自动修复会处理：**
- `durableId` >= 0x7FFFFFFF（重新生成有效 ID）
- `<w:t>` 含空白字符却缺少 `xml:space="preserve"`

**自动修复无法处理：**
- 格式错误的 XML、非法的元素嵌套、缺失的关系、违反 schema 的问题

### 常见陷阱

- **替换整个 `<w:r>` 元素**：添加修订时，把整个 `<w:r>...</w:r>` 块替换为并列的 `<w:del>...<w:ins>...`。不要把修订标签注入 run 内部。
- **保留 `<w:rPr>` 格式**：把原 run 的 `<w:rPr>` 块复制到你新建的修订 run 中，以保持加粗、字号等格式。

---

## XML 参考

### Schema 合规

- **`<w:pPr>` 中的元素顺序**：`<w:pStyle>`、`<w:numPr>`、`<w:spacing>`、`<w:ind>`、`<w:jc>`，`<w:rPr>` 放在最后
- **空白字符**：`<w:t>` 含首尾空格时要加上 `xml:space="preserve"`
- **RSID**：必须是 8 位十六进制（例如 `00AB1234`）

### 修订

**插入：**
```xml
<w:ins w:id="1" w:author="Claude" w:date="2025-01-01T00:00:00Z">
  <w:r><w:t>inserted text</w:t></w:r>
</w:ins>
```

**删除：**
```xml
<w:del w:id="2" w:author="Claude" w:date="2025-01-01T00:00:00Z">
  <w:r><w:delText>deleted text</w:delText></w:r>
</w:del>
```

**在 `<w:del>` 内部**：用 `<w:delText>` 代替 `<w:t>`，用 `<w:delInstrText>` 代替 `<w:instrText>`。

**最小化标记**——只标记发生变化的内容：
```xml
<!-- 把 "30 days" 改为 "60 days" -->
<w:r><w:t>The term is </w:t></w:r>
<w:del w:id="1" w:author="Claude" w:date="...">
  <w:r><w:delText>30</w:delText></w:r>
</w:del>
<w:ins w:id="2" w:author="Claude" w:date="...">
  <w:r><w:t>60</w:t></w:r>
</w:ins>
<w:r><w:t> days.</w:t></w:r>
```

**删除整个段落/列表项**——移除段落中的全部内容时，还要把段落标记一并标记为删除，使其与下一段落合并。在 `<w:pPr><w:rPr>` 内加入 `<w:del/>`：
```xml
<w:p>
  <w:pPr>
    <w:numPr>...</w:numPr>  <!-- 如存在列表编号则保留 -->
    <w:rPr>
      <w:del w:id="1" w:author="Claude" w:date="2025-01-01T00:00:00Z"/>
    </w:rPr>
  </w:pPr>
  <w:del w:id="2" w:author="Claude" w:date="2025-01-01T00:00:00Z">
    <w:r><w:delText>Entire paragraph content being deleted...</w:delText></w:r>
  </w:del>
</w:p>
```
若 `<w:pPr><w:rPr>` 中没有 `<w:del/>`，接受修订后会留下一个空的段落/列表项。

**拒绝其他作者的插入**——把删除嵌套在对方的插入元素内部：
```xml
<w:ins w:author="Jane" w:id="5">
  <w:del w:author="Claude" w:id="10">
    <w:r><w:delText>their inserted text</w:delText></w:r>
  </w:del>
</w:ins>
```

**恢复其他作者的删除**——在其后添加插入（不要改动对方的删除）：
```xml
<w:del w:author="Jane" w:id="5">
  <w:r><w:delText>deleted text</w:delText></w:r>
</w:del>
<w:ins w:author="Claude" w:id="10">
  <w:r><w:t>deleted text</w:t></w:r>
</w:ins>
```

### 批注

运行 `comment.py`（见步骤 2）后，向 document.xml 添加标记。回复时使用 `--parent` 参数，并把标记嵌套在父批注标记的内部。

**关键：`<w:commentRangeStart>` 和 `<w:commentRangeEnd>` 与 `<w:r>` 是同级元素，绝不能放在 `<w:r>` 内部。**

```xml
<!-- 批注标记是 w:p 的直接子元素，绝不能位于 w:r 内部 -->
<w:commentRangeStart w:id="0"/>
<w:del w:id="1" w:author="Claude" w:date="2025-01-01T00:00:00Z">
  <w:r><w:delText>deleted</w:delText></w:r>
</w:del>
<w:r><w:t> more text</w:t></w:r>
<w:commentRangeEnd w:id="0"/>
<w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="0"/></w:r>

<!-- 批注 0，其中嵌套回复 1 -->
<w:commentRangeStart w:id="0"/>
  <w:commentRangeStart w:id="1"/>
  <w:r><w:t>text</w:t></w:r>
  <w:commentRangeEnd w:id="1"/>
<w:commentRangeEnd w:id="0"/>
<w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="0"/></w:r>
<w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="1"/></w:r>
```

### 图片

1. 将图片文件添加到 `word/media/`
2. 在 `word/_rels/document.xml.rels` 中添加关系：
```xml
<Relationship Id="rId5" Type=".../image" Target="media/image1.png"/>
```
3. 在 `[Content_Types].xml` 中添加内容类型：
```xml
<Default Extension="png" ContentType="image/png"/>
```
4. 在 document.xml 中引用：
```xml
<w:drawing>
  <wp:inline>
    <wp:extent cx="914400" cy="914400"/>  <!-- EMU：914400 = 1 英寸 -->
    <a:graphic>
      <a:graphicData uri=".../picture">
        <pic:pic>
          <pic:blipFill><a:blip r:embed="rId5"/></pic:blipFill>
        </pic:pic>
      </a:graphicData>
    </a:graphic>
  </wp:inline>
</w:drawing>
```

---

## 依赖

- **pandoc**：文本提取
- **docx**：`npm install -g docx`（新建文档）
- **LibreOffice**：PDF 转换（沙箱环境通过 `scripts/office/soffice.py` 自动配置）
- **Poppler**：用 `pdftoppm` 生成图片
