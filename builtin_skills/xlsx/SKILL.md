---
name: xlsx
description: "只要电子表格文件是任务的主要输入或输出，就使用本技能。即用户想要：打开、读取、编辑或修复现有的 .xlsx、.xlsm、.csv 或 .tsv 文件（例如添加列、计算公式、设置格式、制作图表、清洗杂乱数据）；从零或其他数据源新建电子表格；或在表格文件格式之间转换。当用户以名称或路径提到某个电子表格文件时——哪怕很随意（比如“我下载文件夹里那个 xlsx”）——并想对它做些处理或基于它产出内容时，尤其应当触发。把杂乱的表格数据文件（畸形行、错位表头、垃圾数据）清洗或重组成规范的电子表格时也应触发。交付物必须是电子表格文件。当主要交付物是 Word 文档、HTML 报告、独立 Python 脚本、数据库管道或 Google Sheets API 集成时不要触发，即使其中涉及表格数据。"
license: Proprietary. LICENSE.txt has complete terms
---

# 产出要求

## 所有 Excel 文件

### 专业字体
- 除非用户另有要求，所有交付物均使用统一、专业的字体（如 Arial、Times New Roman）

### 零公式错误
- 每个 Excel 模型交付时必须做到零公式错误（#REF!、#DIV/0!、#VALUE!、#N/A、#NAME?）

### 保留现有模板（更新模板时）
- 修改文件前先研究，并严格遵循现有的格式、样式和惯例
- 绝不把标准化格式强加给已有既定模式的文件
- 现有模板惯例永远优先于本指南

## 财务模型

### 颜色编码规范
除非用户或现有模板另有说明

#### 行业标准颜色惯例
- **蓝色文字（RGB: 0,0,255）**：硬编码输入，以及用户会按情景调整的数字
- **黑色文字（RGB: 0,0,0）**：所有公式与计算
- **绿色文字（RGB: 0,128,0）**：引用同一工作簿中其他工作表的链接
- **红色文字（RGB: 255,0,0）**：指向其他文件的外部链接
- **黄色背景（RGB: 255,255,0）**：需要关注的关键假设或需要更新的单元格

### 数字格式规范

#### 必须遵守的格式规则
- **年份**：格式化为文本字符串（如 "2024" 而非 "2,024"）
- **货币**：使用 $#,##0 格式；务必在表头标明单位（"Revenue ($mm)"）
- **零值**：用数字格式把所有零显示为 "-"，百分比也不例外（如 "$#,##0;($#,##0);-"）
- **百分比**：默认使用 0.0% 格式（一位小数）
- **倍数**：估值倍数（EV/EBITDA、P/E）用 0.0x 格式
- **负数**：用括号 (123) 而非负号 -123

### 公式构建规则

#### 假设的位置
- 把所有假设（增长率、利润率、倍数等）放进独立的假设单元格
- 公式中使用单元格引用，而非硬编码数值
- 示例：用 =B5*(1+$B$6) 而不是 =B5*1.05

#### 公式错误预防
- 核对所有单元格引用正确无误
- 检查范围中的差一错误
- 确保所有预测期间的公式保持一致
- 用边界情况测试（零值、负数）
- 确认不存在意外的循环引用

#### 硬编码值的文档要求
- 添加批注，或写入旁边单元格（若位于表格末尾）。格式："Source: [System/Document], [Date], [Specific Reference], [URL if applicable]"
- 示例：
  - "Source: Company 10-K, FY2024, Page 45, Revenue Note, [SEC EDGAR URL]"
  - "Source: Company 10-Q, Q2 2025, Exhibit 99.1, [SEC EDGAR URL]"
  - "Source: Bloomberg Terminal, 8/15/2025, AAPL US Equity"
  - "Source: FactSet, 8/20/2025, Consensus Estimates Screen"

# XLSX 的创建、编辑与分析

## 概述

用户可能要求你创建、编辑或分析 .xlsx 文件的内容。针对不同任务，你可以选用不同的工具和工作流。

## 重要前提

**公式重算需要 LibreOffice**：可以假定已安装 LibreOffice，用于通过 `scripts/recalc.py` 脚本重算公式值。该脚本在首次运行时会自动配置 LibreOffice，包括在 Unix 套接字受限的沙箱环境中（由 `scripts/office/soffice.py` 处理）

## 读取与分析数据

### 用 pandas 做数据分析
数据分析、可视化和基础操作请使用 **pandas**，它提供强大的数据处理能力：

```python
import pandas as pd

# 读取 Excel
df = pd.read_excel('file.xlsx')  # 默认：第一个工作表
all_sheets = pd.read_excel('file.xlsx', sheet_name=None)  # 所有工作表，返回字典

# 分析
df.head()      # 预览数据
df.info()      # 列信息
df.describe()  # 统计信息

# 写入 Excel
df.to_excel('output.xlsx', index=False)
```

## Excel 文件工作流

## 关键：使用公式，而非硬编码数值

**永远使用 Excel 公式，不要在 Python 里算好数值再硬编码进去。**这能保证电子表格保持动态、可持续更新。

### ❌ 错误 —— 硬编码计算结果
```python
# 不好：在 Python 中计算并硬编码结果
total = df['Sales'].sum()
sheet['B10'] = total  # 硬编码了 5000

# 不好：在 Python 中计算增长率
growth = (df.iloc[-1]['Revenue'] - df.iloc[0]['Revenue']) / df.iloc[0]['Revenue']
sheet['C5'] = growth  # 硬编码了 0.15

# 不好：用 Python 计算平均值
avg = sum(values) / len(values)
sheet['D20'] = avg  # 硬编码了 42.5
```

### ✅ 正确 —— 使用 Excel 公式
```python
# 好：让 Excel 计算求和
sheet['B10'] = '=SUM(B2:B9)'

# 好：增长率写成 Excel 公式
sheet['C5'] = '=(C4-C2)/C2'

# 好：用 Excel 函数求平均
sheet['D20'] = '=AVERAGE(D2:D19)'
```

这适用于所有计算——合计、百分比、比率、差值等。源数据变化时，电子表格应能重新计算。

## 通用工作流
1. **选择工具**：数据用 pandas，公式/格式用 openpyxl
2. **创建/加载**：新建工作簿或加载现有文件
3. **修改**：添加/编辑数据、公式和格式
4. **保存**：写入文件
5. **重算公式（只要用了公式就是必做）**：使用 scripts/recalc.py 脚本
   ```bash
   python scripts/recalc.py output.xlsx
   ```
6. **验证并修复错误**：
   - 脚本返回包含错误详情的 JSON
   - 若 `status` 为 `errors_found`，查看 `error_summary` 了解具体错误类型和位置
   - 修复识别出的错误并再次重算
   - 常见待修复错误：
     - `#REF!`：无效的单元格引用
     - `#DIV/0!`：除以零
     - `#VALUE!`：公式中数据类型错误
     - `#NAME?`：无法识别的公式名

### 新建 Excel 文件

```python
# 使用 openpyxl 处理公式与格式
from openpyxl import Workbook
from openpyxl.styles import Font, PatternFill, Alignment

wb = Workbook()
sheet = wb.active

# 添加数据
sheet['A1'] = 'Hello'
sheet['B1'] = 'World'
sheet.append(['Row', 'of', 'data'])

# 添加公式
sheet['B2'] = '=SUM(A1:A10)'

# 设置格式
sheet['A1'].font = Font(bold=True, color='FF0000')
sheet['A1'].fill = PatternFill('solid', start_color='FFFF00')
sheet['A1'].alignment = Alignment(horizontal='center')

# 列宽
sheet.column_dimensions['A'].width = 20

wb.save('output.xlsx')
```

### 编辑现有 Excel 文件

```python
# 使用 openpyxl 保留公式与格式
from openpyxl import load_workbook

# 加载现有文件
wb = load_workbook('existing.xlsx')
sheet = wb.active  # 或用 wb['SheetName'] 指定工作表

# 处理多个工作表
for sheet_name in wb.sheetnames:
    sheet = wb[sheet_name]
    print(f"Sheet: {sheet_name}")

# 修改单元格
sheet['A1'] = 'New Value'
sheet.insert_rows(2)  # 在第 2 行位置插入行
sheet.delete_cols(3)  # 删除第 3 列

# 新建工作表
new_sheet = wb.create_sheet('NewSheet')
new_sheet['A1'] = 'Data'

wb.save('modified.xlsx')
```

## 重算公式

openpyxl 创建或修改的 Excel 文件中，公式只是字符串，不含计算结果。请使用提供的 `scripts/recalc.py` 脚本重算公式：

```bash
python scripts/recalc.py <excel_file> [timeout_seconds]
```

示例：
```bash
python scripts/recalc.py output.xlsx 30
```

该脚本会：
- 首次运行时自动配置 LibreOffice 宏
- 重算所有工作表中的全部公式
- 扫描所有单元格中的 Excel 错误（#REF!、#DIV/0! 等）
- 返回包含详细错误位置和计数的 JSON
- 在 Linux 和 macOS 上均可用

## 公式验证清单

快速检查，确保公式工作正常：

### 基础验证
- [ ] **测试 2-3 个样例引用**：搭建完整模型前先验证它们取值正确
- [ ] **列映射**：确认 Excel 列号对应正确（如第 64 列是 BL 而非 BK）
- [ ] **行偏移**：记住 Excel 行号从 1 开始（DataFrame 第 5 行 = Excel 第 6 行）

### 常见陷阱
- [ ] **NaN 处理**：用 `pd.notna()` 检查空值
- [ ] **靠右的列**：财年数据常在第 50 列以后
- [ ] **多处匹配**：搜索所有出现位置，而不只是第一处
- [ ] **除以零**：在公式中使用 `/` 前先检查分母（#DIV/0!）
- [ ] **引用错误**：核对所有单元格引用都指向预期单元格（#REF!）
- [ ] **跨表引用**：链接工作表时使用正确格式（Sheet1!A1）

### 公式测试策略
- [ ] **从小做起**：先在 2-3 个单元格上测试公式，再大规模应用
- [ ] **验证依赖**：检查公式引用的单元格都存在
- [ ] **测试边界情况**：包含零、负数和超大值

### 解读 scripts/recalc.py 的输出
脚本返回包含错误详情的 JSON：
```json
{
  "status": "success",           // 或 "errors_found"
  "total_errors": 0,              // 错误总数
  "total_formulas": 42,           // 文件中的公式数量
  "error_summary": {              // 仅在发现错误时出现
    "#REF!": {
      "count": 2,
      "locations": ["Sheet1!B5", "Sheet1!C10"]
    }
  }
}
```

## 最佳实践

### 库的选择
- **pandas**：最适合数据分析、批量操作和简单数据导出
- **openpyxl**：最适合复杂格式、公式和 Excel 特有功能

### 使用 openpyxl
- 单元格索引从 1 开始（row=1、column=1 指单元格 A1）
- 用 `data_only=True` 读取计算结果：`load_workbook('file.xlsx', data_only=True)`
- **警告**：以 `data_only=True` 打开再保存的话，公式会被替换为数值并永久丢失
- 大文件：读取用 `read_only=True`，写入用 `write_only=True`
- 公式会保留但不会被求值——用 scripts/recalc.py 更新数值

### 使用 pandas
- 指定数据类型以避免类型推断问题：`pd.read_excel('file.xlsx', dtype={'id': str})`
- 大文件可只读取指定列：`pd.read_excel('file.xlsx', usecols=['A', 'C', 'E'])`
- 正确处理日期：`pd.read_excel('file.xlsx', parse_dates=['date_column'])`

## 代码风格指南
**重要**：为 Excel 操作生成 Python 代码时：
- 编写精简的 Python 代码，不加多余的注释
- 避免冗长的变量名和重复操作
- 避免不必要的 print 语句

**对于 Excel 文件本身**：
- 为含复杂公式或重要假设的单元格添加批注
- 为硬编码值记录数据来源
- 为关键计算和模型小节添加说明
