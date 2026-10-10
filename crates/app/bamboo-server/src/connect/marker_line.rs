//! 行级出站控制标记的解析（`[SEND_FILE: …]` / `[SEND_VOICE: …]` /
//! `[SET_MODEL: …]` 等）。
//!
//! 标记约定由 bridge 的渠道能力前言教给模型，网关在回复出口按行剥离并
//! 执行。解析必须宽容：模型在中文提示词/中文推理语境下经常把标记写成
//! 全角标点（`［SEND_FILE：…］`、`【SEND_FILE：…】`）或漏掉冒号后的
//! 空格——严格半角前缀匹配会整类媒体/命令投递静默失效（标记行被当
//! 普通文本原样发给用户）。此模块把"什么算标记行"集中成一条规则，
//! wechat / bridge / voice 的出口解析共用。

/// 全角空格（U+3000），中文输入法常见。
const FULLWIDTH_SPACE: char = '\u{3000}';

fn is_marker_space(c: char) -> bool {
    c == ' ' || c == '\t' || c == FULLWIDTH_SPACE
}

/// 闭括号后可能跟的句读（中文句式里模型常写 `[SEND_FILE: /a.png]。`）。
/// 只剥行尾、且在闭括号之后——内容里的句读不受影响。
fn is_trailing_punctuation(c: char) -> bool {
    matches!(
        c,
        '。' | '．' | '.' | '，' | ',' | '、' | '；' | ';' | '！' | '!' | '？' | '?'
    )
}

/// 尝试把一整行解析为 `name` 标记行；命中时返回标记内容（首尾空白已去，
/// 其余原样保留——内容里的全角字符、路径、空格都不改写）。
///
/// 宽容点（相对旧版严格 `"[NAME: "` 前缀 + `']'` 后缀）：
/// - 开括号 `[`／`［`／`【` 等价，闭括号 `]`／`］`／`】` 等价（不要求配对）；
/// - 冒号半角 `:` 与全角 `：` 等价，其后空格（含全角空格）可省略；
/// - 闭括号后跟句读（`。`、`，` 等，中文句式里很常见）忽略；
/// - 标记名大小写不敏感；
/// - 行首尾空白忽略。
///
/// 仍然严格的部分：标记必须独占一行；未闭合或内容为空视为未命中
/// （原样保留在可见文本里，绝不吞内容）。
pub(crate) fn strip_marker_line<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    debug_assert!(
        name.is_ascii() && !name.is_empty(),
        "marker name must be ASCII"
    );
    let trimmed = line.trim().trim_end_matches(is_trailing_punctuation);
    let after_open = trimmed
        .strip_prefix('[')
        .or_else(|| trimmed.strip_prefix('［'))
        .or_else(|| trimmed.strip_prefix('【'))?;
    // `get` 在非字符边界返回 None——name 是纯 ASCII，先按字节长度切头部。
    let head = after_open.get(..name.len())?;
    if !head.eq_ignore_ascii_case(name) {
        return None;
    }
    let after_name = &after_open[name.len()..];
    let after_spaces = after_name.trim_start_matches(is_marker_space);
    let after_colon = after_spaces
        .strip_prefix(':')
        .or_else(|| after_spaces.strip_prefix('：'))?;
    let content_zone = after_colon.trim_start_matches(is_marker_space);
    let content = content_zone
        .strip_suffix(']')
        .or_else(|| content_zone.strip_suffix('］'))
        .or_else(|| content_zone.strip_suffix('】'))?
        .trim();
    if content.is_empty() {
        None
    } else {
        Some(content)
    }
}

/// 提取回复文本中全部 `name` 标记行：返回（剥掉标记行后的可见文本，
/// 标记内容列表，按出现顺序）。SEND_FILE/SEND_VOICE 等单名字提取共用
/// 这一份，避免每个标记手写一遍同样的行级循环。
pub(crate) fn extract_marker_lines(text: &str, name: &str) -> (String, Vec<String>) {
    let mut values = Vec::new();
    let mut kept: Vec<&str> = Vec::new();
    for line in text.lines() {
        if let Some(value) = strip_marker_line(line, name) {
            values.push(value.to_string());
            continue;
        }
        kept.push(line);
    }
    (kept.join("\n").trim().to_string(), values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_halfwidth_form_still_matches() {
        assert_eq!(
            strip_marker_line("[SEND_FILE: /tmp/a.png]", "SEND_FILE"),
            Some("/tmp/a.png")
        );
        assert_eq!(
            strip_marker_line("  [SEND_FILE: /tmp/a.png]  ", "SEND_FILE"),
            Some("/tmp/a.png")
        );
        assert_eq!(
            strip_marker_line("[SET_MODEL: gpt-x]", "SET_MODEL"),
            Some("gpt-x")
        );
    }

    #[test]
    fn fullwidth_brackets_and_colon_match() {
        assert_eq!(
            strip_marker_line("［SEND_FILE：C:\\报告\\季报.docx］", "SEND_FILE"),
            Some("C:\\报告\\季报.docx")
        );
        assert_eq!(
            strip_marker_line("【SEND_FILE：/tmp/a.png】", "SEND_FILE"),
            Some("/tmp/a.png")
        );
        assert_eq!(
            strip_marker_line("[SEND_VOICE：明天三点开会］", "SEND_VOICE"),
            Some("明天三点开会")
        );
    }

    #[test]
    fn missing_space_and_fullwidth_space_match() {
        assert_eq!(
            strip_marker_line("[SEND_FILE:/tmp/a.png]", "SEND_FILE"),
            Some("/tmp/a.png")
        );
        assert_eq!(
            strip_marker_line("[SEND_FILE:\u{3000}/tmp/a.png]", "SEND_FILE"),
            Some("/tmp/a.png")
        );
        assert_eq!(
            strip_marker_line("[SEND_FILE:  /tmp/a.png]", "SEND_FILE"),
            Some("/tmp/a.png")
        );
    }

    #[test]
    fn trailing_punctuation_after_closing_bracket_is_ignored() {
        // 中文句式常见：闭括号后跟句读。内容内部的句读不受影响。
        assert_eq!(
            strip_marker_line("[SEND_FILE: /tmp/a.png]。", "SEND_FILE"),
            Some("/tmp/a.png")
        );
        assert_eq!(
            strip_marker_line("［SEND_VOICE：念到这里。］，", "SEND_VOICE"),
            Some("念到这里。")
        );
    }

    #[test]
    fn marker_name_is_case_insensitive() {
        assert_eq!(
            strip_marker_line("[send_file: /tmp/a.png]", "SEND_FILE"),
            Some("/tmp/a.png")
        );
    }

    #[test]
    fn content_is_preserved_verbatim_including_fullwidth_colon() {
        // 语音文本内部的全角冒号/全角括号绝不能被"规范化"掉。
        assert_eq!(
            strip_marker_line("[SEND_VOICE: 提醒：三点开会（带笔记本）]", "SEND_VOICE"),
            Some("提醒：三点开会（带笔记本）")
        );
    }

    #[test]
    fn unclosed_or_empty_markers_do_not_match() {
        assert_eq!(strip_marker_line("[SEND_FILE: 没闭合", "SEND_FILE"), None);
        assert_eq!(strip_marker_line("[SEND_FILE: ]", "SEND_FILE"), None);
        assert_eq!(strip_marker_line("[SEND_FILE:]", "SEND_FILE"), None);
        assert_eq!(strip_marker_line("[]", "SEND_FILE"), None);
    }

    #[test]
    fn wrong_marker_name_or_mid_line_does_not_match() {
        assert_eq!(
            strip_marker_line("[SEND_FILES: /tmp/a.png]", "SEND_FILE"),
            None
        );
        // 标记必须独占一行：正文混在同一行不认。
        assert_eq!(
            strip_marker_line("请看 [SEND_FILE: /tmp/a.png]", "SEND_FILE"),
            None
        );
        assert_eq!(strip_marker_line("SET_MODEL: gpt-x", "SET_MODEL"), None);
    }

    #[test]
    fn extract_marker_lines_strips_in_order_and_keeps_visible_text() {
        let text =
            "报告如下：\n[SEND_FILE: /tmp/a.png]\n正文说明\n［SEND_FILE：/tmp/b.png］\n以上。";
        let (visible, files) = extract_marker_lines(text, "SEND_FILE");
        assert_eq!(visible, "报告如下：\n正文说明\n以上。");
        assert_eq!(
            files,
            vec!["/tmp/a.png".to_string(), "/tmp/b.png".to_string()]
        );
        // 无标记时原样返回。
        let (visible, files) = extract_marker_lines("普通回复", "SEND_FILE");
        assert_eq!(visible, "普通回复");
        assert!(files.is_empty());
    }
}
