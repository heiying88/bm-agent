**关键：必须按顺序完成这些步骤，不要跳过步骤直接写代码。**

如果需要填写 PDF 表单，先检查该 PDF 是否带有可填写的表单域。在本文件所在目录下运行此脚本：
 `python scripts/check_fillable_fields <file.pdf>`，然后根据结果进入“可填写表单域”或“不可填写表单域”部分，按照相应说明操作。

# 可填写表单域
如果 PDF 带有可填写的表单域：
- 在本文件所在目录下运行此脚本：`python scripts/extract_form_field_info.py <input.pdf> <field_info.json>`。它会生成一个 JSON 文件，其中包含如下格式的字段列表：
```
[
  {
    "field_id": (字段的唯一 ID),
    "page": (页码，从 1 开始),
    "rect": ([左, 下, 右, 上] 边界框，PDF 坐标，y=0 为页面底端),
    "type": ("text"、"checkbox"、"radio_group" 或 "choice"),
  },
  // 复选框带有 "checked_value" 和 "unchecked_value" 属性：
  {
    "field_id": (字段的唯一 ID),
    "page": (页码，从 1 开始),
    "type": "checkbox",
    "checked_value": (将字段设为该值即可勾选复选框),
    "unchecked_value": (将字段设为该值即可取消勾选),
  },
  // 单选按钮组带有 "radio_options" 列表，列出所有可选项。
  {
    "field_id": (字段的唯一 ID),
    "page": (页码，从 1 开始),
    "type": "radio_group",
    "radio_options": [
      {
        "value": (将字段设为该值即可选中该单选项),
        "rect": (该选项单选按钮的边界框)
      },
      // 其他单选项
    ]
  },
  // 下拉列表域带有 "choice_options" 列表，列出所有可选项：
  {
    "field_id": (字段的唯一 ID),
    "page": (页码，从 1 开始),
    "type": "choice",
    "choice_options": [
      {
        "value": (将字段设为该值即可选中该选项),
        "text": (该选项的显示文本)
      },
      // 其他选项
    ],
  }
]
```
- 用此脚本（在本文件所在目录下运行）将 PDF 转换为 PNG（每页一张图像）：
`python scripts/convert_pdf_to_images.py <file.pdf> <output_directory>`
然后分析这些图像，确定每个表单域的用途（务必将边界框的 PDF 坐标转换为图像坐标）。
- 按此格式创建 `field_values.json` 文件，填入每个字段要写入的值：
```
[
  {
    "field_id": "last_name", // 必须与 `extract_form_field_info.py` 中的 field_id 一致
    "description": "The user's last name",
    "page": 1, // 必须与 field_info.json 中的 "page" 值一致
    "value": "Simpson"
  },
  {
    "field_id": "Checkbox12",
    "description": "Checkbox to be checked if the user is 18 or over",
    "page": 1,
    "value": "/On" // 若为复选框，使用其 "checked_value" 值来勾选；若为单选按钮组，使用 "radio_options" 中的某个 "value" 值。
  },
  // 更多字段
]
```
- 在本文件所在目录下运行 `fill_fillable_fields.py` 脚本，生成填写好的 PDF：
`python scripts/fill_fillable_fields.py <input pdf> <field_values.json> <output pdf>`
此脚本会校验你提供的字段 ID 和值是否有效；如果它输出错误信息，请修正相应字段后重试。

# 不可填写表单域
如果 PDF 没有可填写的表单域，就要通过添加文本注释的方式填写。先尝试从 PDF 结构中提取坐标（更精确），必要时再回退到视觉估算。

## 第 1 步：优先尝试结构提取

运行此脚本，提取文本标签、线条和复选框及其精确的 PDF 坐标：
`python scripts/extract_form_structure.py <input.pdf> form_structure.json`

这会生成一个 JSON 文件，其中包含：
- **labels**：每个文本元素及其精确坐标（x0、top、x1、bottom，单位为 PDF 点）
- **lines**：界定各行边界的水平线
- **checkboxes**：作为复选框的小方形矩形（附中心坐标）
- **row_boundaries**：由水平线计算得到的行上/下边界位置

**检查结果**：如果 `form_structure.json` 包含有意义的标签（与表单域对应的文本元素），使用**方案 A：基于结构的坐标**；如果 PDF 是扫描版/图像版、标签很少或没有，使用**方案 B：视觉估算**。

---

## 方案 A：基于结构的坐标（首选）

当 `extract_form_structure.py` 在 PDF 中找到了文本标签时，使用此方案。

### A.1：分析结构

阅读 form_structure.json 并识别：

1. **标签组**：相邻的、共同构成一个标签的文本元素（例如 “Last” + “Name”）
2. **行结构**：`top` 值相近的标签位于同一行
3. **字段列**：填写区从标签结束处开始（x0 = label.x1 + 间距）
4. **复选框**：直接使用结构中给出的复选框坐标

**坐标系**：PDF 坐标，y=0 位于页面顶部，y 向下递增。

### A.2：检查遗漏元素

结构提取可能检测不到全部表单元素。常见情况：
- **圆形复选框**：只有方形矩形才会被识别为复选框
- **复杂图形**：装饰性元素或非标准表单控件
- **褪色或浅色元素**：可能无法被提取

如果在 PDF 图像中看到的表单域未出现在 form_structure.json 中，就需要对这些特定域使用**视觉分析**（参见下文的“混合方案”）。

### A.3：用 PDF 坐标创建 fields.json

根据提取到的结构，为每个字段计算填写区坐标：

**文本域：**
- 填写区 x0 = 标签 x1 + 5（标签后留一小段间距）
- 填写区 x1 = 下一个标签的 x0，或行边界
- 填写区 top = 与标签 top 相同
- 填写区 bottom = 下方的行边界线，或标签 bottom + row_height

**复选框：**
- 直接使用 form_structure.json 中的复选框矩形坐标
- entry_bounding_box = [checkbox.x0, checkbox.top, checkbox.x1, checkbox.bottom]

使用 `pdf_width` 和 `pdf_height` 创建 fields.json（表示使用 PDF 坐标）：
```json
{
  "pages": [
    {"page_number": 1, "pdf_width": 612, "pdf_height": 792}
  ],
  "form_fields": [
    {
      "page_number": 1,
      "description": "Last name entry field",
      "field_label": "Last Name",
      "label_bounding_box": [43, 63, 87, 73],
      "entry_bounding_box": [92, 63, 260, 79],
      "entry_text": {"text": "Smith", "font_size": 10}
    },
    {
      "page_number": 1,
      "description": "US Citizen Yes checkbox",
      "field_label": "Yes",
      "label_bounding_box": [260, 200, 280, 210],
      "entry_bounding_box": [285, 197, 292, 205],
      "entry_text": {"text": "X"}
    }
  ]
}
```

**重要**：使用 `pdf_width`/`pdf_height`，并直接采用 form_structure.json 中的坐标。

### A.4：校验边界框

填写前，先检查边界框是否有错误：
`python scripts/check_bounding_boxes.py fields.json`

它会检查相交的边界框以及相对于字号过小的填写框。请先修复报告的错误，再进行填写。

---

## 方案 B：视觉估算（回退方案）

当 PDF 是扫描版/图像版、结构提取未找到可用文本标签时（例如所有文本都显示为 “(cid:X)” 模式），使用此方案。

### B.1：将 PDF 转换为图像

`python scripts/convert_pdf_to_images.py <input.pdf> <images_dir/>`

### B.2：初步识别字段

逐页查看图像，识别表单区域，并对字段位置做出**粗略估计**：
- 表单域标签及其大致位置
- 填写区（线条、方框或用于输入文本的空白区域）
- 复选框及其大致位置

对每个字段，记下大致的像素坐标（此阶段无需精确）。

### B.3：放大细化（对精度至关重要）

对每个字段，在估计位置附近裁剪一块区域，以精确细化坐标。

**使用 ImageMagick 裁剪放大图：**
```bash
magick <page_image> -crop <width>x<height>+<x>+<y> +repage <crop_output.png>
```

其中：
- `<x>, <y>` = 裁剪区域左上角（取粗略估计值减去留白）
- `<width>, <height>` = 裁剪区域大小（字段区域四周各加约 50px 留白）

**示例：**细化一个估计位于 (100, 150) 附近的 “Name” 字段：
```bash
magick images_dir/page_1.png -crop 300x80+50+120 +repage crops/name_field.png
```

（注意：如果 `magick` 命令不可用，可尝试参数相同的 `convert` 命令）。

**查看裁剪出的图像**，确定精确坐标：
1. 确定填写区起始处的精确像素（标签之后）
2. 确定填写区结束的位置（下一个字段或页面边缘之前）
3. 确定填写线/填写框的顶部和底部

**将裁剪坐标换算回完整图像坐标：**
- full_x = crop_x + crop_offset_x
- full_y = crop_y + crop_offset_y

示例：若裁剪起点为 (50, 120)，填写框在裁剪图内的起点为 (52, 18)：
- entry_x0 = 52 + 50 = 102
- entry_top = 18 + 120 = 138

**对每个字段重复此过程**，并尽可能将相邻字段合并到一次裁剪中。

### B.4：用细化后的坐标创建 fields.json

使用 `image_width` 和 `image_height` 创建 fields.json（表示使用图像坐标）：
```json
{
  "pages": [
    {"page_number": 1, "image_width": 1700, "image_height": 2200}
  ],
  "form_fields": [
    {
      "page_number": 1,
      "description": "Last name entry field",
      "field_label": "Last Name",
      "label_bounding_box": [120, 175, 242, 198],
      "entry_bounding_box": [255, 175, 720, 218],
      "entry_text": {"text": "Smith", "font_size": 10}
    }
  ]
}
```

**重要**：使用 `image_width`/`image_height` 以及放大分析得到的精确像素坐标。

### B.5：校验边界框

填写前，先检查边界框是否有错误：
`python scripts/check_bounding_boxes.py fields.json`

它会检查相交的边界框以及相对于字号过小的填写框。请先修复报告的错误，再进行填写。

---

## 混合方案：结构 + 视觉

当结构提取对大多数字段有效、但遗漏了某些元素时（例如圆形复选框、非常规表单控件），使用此方案。

1. 对 form_structure.json 中检测到的字段，**使用方案 A**
2. **将 PDF 转换为图像**，对遗漏字段做视觉分析
3. 对遗漏字段**使用放大细化**（来自方案 B）
4. **合并坐标**：来自结构提取的字段使用 `pdf_width`/`pdf_height`；视觉估算的字段必须将图像坐标换算为 PDF 坐标：
   - pdf_x = image_x * (pdf_width / image_width)
   - pdf_y = image_y * (pdf_height / image_height)
5. 在 fields.json 中**使用统一坐标系**——将所有坐标都换算为使用 `pdf_width`/`pdf_height` 的 PDF 坐标

---

## 第 2 步：填写前校验

**填写前务必校验边界框：**
`python scripts/check_bounding_boxes.py fields.json`

它会检查：
- 相交的边界框（会导致文字重叠）
- 相对于指定字号过小的填写框

继续之前，请先修复 fields.json 中报告的所有错误。

## 第 3 步：填写表单

填写脚本会自动检测坐标系并处理转换：
`python scripts/fill_pdf_form_with_annotations.py <input.pdf> fields.json <output.pdf>`

## 第 4 步：验证输出

将填写后的 PDF 转换为图像，验证文字位置：
`python scripts/convert_pdf_to_images.py <output.pdf> <verify_images/>`

如果文字位置不对：
- **方案 A**：检查是否在配合 `pdf_width`/`pdf_height` 使用 form_structure.json 的 PDF 坐标
- **方案 B**：检查图像尺寸是否匹配、坐标是否为准确的像素值
- **混合方案**：确保视觉估算字段的坐标换算正确
