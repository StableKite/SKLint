use crate::config::EffectiveConfig;
use crate::diagnostic::{Diagnostic, Fix, Span};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

#[derive(Debug, Clone)]
struct LineInfo {
    no: usize,
    text: String,
    code: String,
    indent: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefKind {
    Class,
    Function,
}

#[derive(Debug, Clone)]
struct DefInfo {
    kind: DefKind,
    name: String,
    start: usize,
    end: usize,
    indent: usize,
    parent: Option<usize>,
}

pub fn run_syntax_rules(path: &Path, source: &str, config: &EffectiveConfig) -> Vec<Diagnostic> {
    let display_path = path.display().to_string();
    let lines = line_infos(source);
    let defs = parse_defs(&lines);
    let mut diagnostics = Vec::new();

    if config.is_enabled("SK201") {
        run_print_statement(&display_path, &lines, &mut diagnostics);
    }
    if config.is_enabled("SK401") {
        run_assignment_spacing(&display_path, &lines, &mut diagnostics);
    }
    if config.is_enabled("SK403") {
        run_multiline_bracket_layout(&display_path, &lines, &mut diagnostics);
    }
    if config.is_enabled("SK404") {
        run_trailing_comma(&display_path, &lines, &mut diagnostics);
    }
    if config.is_enabled("SK502") {
        run_from_import_only(&display_path, source, &lines, &mut diagnostics);
    }
    if config.is_enabled("SK503") {
        run_os_name(&display_path, &lines, &mut diagnostics);
    }
    if config.is_enabled("SK504") {
        run_sys_platform_import(&display_path, source, &lines, &mut diagnostics);
    }
    if config.is_enabled("SK505") {
        run_definition_order(&display_path, &lines, &defs, &mut diagnostics);
    }
    if config.is_enabled("SK509") {
        run_special_method_order(&display_path, &defs, &mut diagnostics);
    }
    if config.is_enabled("SK506") {
        run_try_blocks(&display_path, &lines, &mut diagnostics);
    }
    if config.is_enabled("SK507") {
        run_raise_hot_path(&display_path, &lines, &defs, &mut diagnostics);
    }
    if config.is_enabled("SK508") {
        run_future_annotations_import(&display_path, source, &lines, &mut diagnostics);
    }
    if config.is_enabled("SK801") {
        run_inline_temp_variable(&display_path, &lines, &mut diagnostics);
    }
    if config.is_enabled("SK802") {
        run_return_ternary(&display_path, &lines, &mut diagnostics);
    }
    if config.is_enabled("SK803") {
        run_loop_comprehension(&display_path, &lines, &mut diagnostics);
    }
    if config.is_enabled("SK804") {
        run_all_tuple(&display_path, source, &lines, &defs, &mut diagnostics);
    }

    diagnostics
}

fn line_infos(source: &str) -> Vec<LineInfo> {
    let raw_lines: Vec<&str> = source.split('\n').collect();
    let code_lines = mask_non_code(source);
    let mut lines: Vec<LineInfo> = raw_lines
        .into_iter()
        .enumerate()
        .map(|(idx, raw)| {
            let text = raw.strip_suffix('\r').unwrap_or(raw).to_string();
            let code = code_lines
                .get(idx)
                .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
                .unwrap_or_default();
            let indent = indent_width(&text);
            LineInfo {
                no: idx + 1,
                text,
                code,
                indent,
            }
        })
        .collect();
    if source.ends_with('\n') {
        lines.pop();
    }
    lines
}

fn mask_non_code(source: &str) -> Vec<String> {
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut idx = 0usize;
    let mut quote: Option<u8> = None;
    let mut triple_quote: Option<u8> = None;
    let mut escape = false;

    while idx < bytes.len() {
        let ch = bytes[idx];

        if let Some(q) = triple_quote {
            if ch == b'\n' {
                out.push('\n');
                idx += 1;
                continue;
            }
            if idx + 2 < bytes.len()
                && bytes[idx] == q
                && bytes[idx + 1] == q
                && bytes[idx + 2] == q
            {
                out.push_str("   ");
                idx += 3;
                triple_quote = None;
                continue;
            }
            out.push(' ');
            idx += 1;
            continue;
        }

        if let Some(q) = quote {
            if ch == b'\n' {
                out.push('\n');
                if escape {
                    escape = false;
                } else {
                    quote = None;
                }
                idx += 1;
                continue;
            }
            if escape {
                escape = false;
            } else if ch == b'\\' {
                escape = true;
            } else if ch == q {
                quote = None;
            }
            out.push(' ');
            idx += 1;
            continue;
        }

        match ch {
            b'#' => {
                while idx < bytes.len() && bytes[idx] != b'\n' {
                    out.push(' ');
                    idx += 1;
                }
            }
            b'\'' | b'"' => {
                if idx + 2 < bytes.len() && bytes[idx + 1] == ch && bytes[idx + 2] == ch {
                    out.push_str("   ");
                    idx += 3;
                    triple_quote = Some(ch);
                } else {
                    out.push('_');
                    idx += 1;
                    quote = Some(ch);
                    escape = false;
                }
            }
            _ => {
                out.push(ch as char);
                idx += 1;
            }
        }
    }

    out.split('\n').map(ToString::to_string).collect()
}

fn strip_comment_and_strings(line: &str) -> String {
    mask_non_code(line).into_iter().next().unwrap_or_default()
}

fn parse_defs(lines: &[LineInfo]) -> Vec<DefInfo> {
    let mut defs = Vec::new();
    for line in lines {
        let trimmed = line.code.trim_start();
        let kind_name = if trimmed.starts_with("class ") {
            parse_name_after_keyword(trimmed, "class ").map(|name| (DefKind::Class, name))
        } else if trimmed.starts_with("def ") {
            parse_name_after_keyword(trimmed, "def ").map(|name| (DefKind::Function, name))
        } else if trimmed.starts_with("async def ") {
            parse_name_after_keyword(trimmed, "async def ").map(|name| (DefKind::Function, name))
        } else {
            None
        };
        let Some((kind, name)) = kind_name else {
            continue;
        };
        let end = block_end(lines, line.no, line.indent);
        defs.push(DefInfo {
            kind,
            name,
            start: line.no,
            end,
            indent: line.indent,
            parent: None,
        });
    }
    for idx in 0..defs.len() {
        let start = defs[idx].start;
        let indent = defs[idx].indent;
        defs[idx].parent = (0..defs.len())
            .filter(|candidate| *candidate != idx)
            .filter(|candidate| defs[*candidate].start < start && start <= defs[*candidate].end)
            .filter(|candidate| defs[*candidate].indent < indent)
            .max_by_key(|candidate| defs[*candidate].indent);
    }
    defs
}

fn parse_name_after_keyword(trimmed: &str, keyword: &str) -> Option<String> {
    let rest = trimmed.strip_prefix(keyword)?.trim_start();
    let name: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn block_end(lines: &[LineInfo], start: usize, indent: usize) -> usize {
    let header_end = header_end_line(lines, start);
    let mut end = lines.len();
    for line_no in header_end + 1..=lines.len() {
        let line = &lines[line_no - 1];
        if line.text.trim().is_empty() {
            continue;
        }
        if line.indent <= indent {
            end = line_no - 1;
            break;
        }
    }
    end
}

fn header_end_line(lines: &[LineInfo], start: usize) -> usize {
    let mut depth = 0usize;
    for line_no in start..=lines.len() {
        let code = &lines[line_no - 1].code;
        for ch in code.chars() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        if depth == 0 && code.trim_end().ends_with(':') {
            return line_no;
        }
    }
    start
}

fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum()
}

fn run_print_statement(display_path: &str, lines: &[LineInfo], diagnostics: &mut Vec<Diagnostic>) {
    for line in lines {
        if is_inside_main_block(line, lines) {
            continue;
        }
        let mut start = 0usize;
        while let Some(pos) = line.code[start..].find("print") {
            let byte = start + pos;
            let before = char_before(&line.code, byte);
            let after = line.code[byte + "print".len()..].chars().next();
            if before.is_none_or(|ch| !is_ident_continue(ch)) && matches!(after, Some('(')) {
                let col = byte_to_column(&line.text, byte);
                diagnostics.push(Diagnostic::new(
                    "SK201",
                    "print calls are forbidden outside if __name__ == \"__main__\" blocks",
                    display_path,
                    Span::new(line.no, col, line.no, col + "print".len()),
                    "warning",
                ));
            }
            start = byte + "print".len();
        }
    }
}

fn is_inside_main_block(line: &LineInfo, lines: &[LineInfo]) -> bool {
    lines
        .iter()
        .take(line.no.saturating_sub(1))
        .rev()
        .filter(|candidate| !candidate.code.trim().is_empty())
        .filter(|candidate| candidate.indent < line.indent)
        .any(|candidate| {
            is_main_guard(candidate.text.trim_start())
                && lines[candidate.no..line.no.saturating_sub(1)]
                    .iter()
                    .filter(|inner| !inner.code.trim().is_empty())
                    .all(|inner| inner.indent > candidate.indent)
        })
}

fn is_main_guard(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|ch| !ch.is_whitespace()).collect();
    compact.starts_with("if__name__==\"__main__\":")
        || compact.starts_with("if__name__=='__main__':")
}

fn run_assignment_spacing(
    display_path: &str,
    lines: &[LineInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for line in lines {
        for (byte_idx, ch) in line.code.char_indices() {
            if ch != '=' || is_assignment_exception(&line.code, byte_idx) {
                continue;
            }
            let before = char_before(&line.code, byte_idx);
            let after = line.code[byte_idx + 1..].chars().next();
            if after.is_none_or(|item| item.is_whitespace())
                && after_code_is_empty(&line.code, byte_idx + 1)
            {
                continue;
            }
            if before.is_some_and(|item| item.is_whitespace())
                && after.is_some_and(|item| item.is_whitespace())
            {
                continue;
            }
            let col = byte_to_column(&line.text, byte_idx);
            let new_line = normalize_operator_on_line(&line.text, byte_idx, 1, "=");
            diagnostics.push(
                Diagnostic::new(
                    "SK401",
                    "Assignment operators must have spaces on both sides",
                    display_path,
                    Span::new(line.no, col, line.no, col + 1),
                    "warning",
                )
                .with_fix(Fix {
                    message: "Normalize spaces around =".to_string(),
                    replacement: new_line,
                    start_line: line.no,
                    start_column: 1,
                    end_line: line.no,
                    end_column: line.text.chars().count() + 1,
                }),
            );
        }
    }
}

fn is_assignment_exception(code: &str, idx: usize) -> bool {
    let before = char_before(code, idx);
    let after = code[idx + 1..].chars().next();
    matches!(
        before,
        Some('!' | '=' | '<' | '>' | ':' | '+' | '-' | '*' | '/' | '%' | '@' | '&' | '|' | '^')
    ) || matches!(after, Some('='))
}

fn after_code_is_empty(code: &str, idx: usize) -> bool {
    code[idx..].trim().is_empty()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn normalize_operator_on_line(line: &str, op_idx: usize, op_len: usize, op: &str) -> String {
    let mut start = op_idx;
    while start > 0 {
        let prev_start = previous_char_start(line, start).unwrap_or(start);
        let prev = line[prev_start..start].chars().next().unwrap_or(' ');
        if prev.is_whitespace() {
            start = prev_start;
        } else {
            break;
        }
    }
    let mut end = op_idx + op_len;
    while end < line.len() {
        let next = line[end..].chars().next().unwrap_or(' ');
        if next.is_whitespace() {
            end += next.len_utf8();
        } else {
            break;
        }
    }
    let mut out = String::new();
    out.push_str(&line[..start]);
    out.push(' ');
    out.push_str(op);
    out.push(' ');
    out.push_str(&line[end..]);
    out
}

fn previous_char_start(text: &str, idx: usize) -> Option<usize> {
    text[..idx].char_indices().last().map(|(pos, _)| pos)
}

fn char_before(text: &str, idx: usize) -> Option<char> {
    previous_char_start(text, idx).and_then(|pos| text[pos..idx].chars().next())
}

fn run_multiline_bracket_layout(
    display_path: &str,
    lines: &[LineInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut stack: Vec<(char, usize, usize)> = Vec::new();
    for line in lines {
        let trimmed = line.code.trim();
        if let Some((_, open_line, _)) = stack.last().copied() {
            if line.no > open_line && !trimmed.is_empty() && !is_pure_closer(trimmed) {
                if let Some((item_start, item_end)) =
                    first_extra_item_after_top_level_comma(&line.code)
                {
                    let col_start = byte_to_column(&line.text, item_start);
                    let col_end = byte_to_column(&line.text, item_end);
                    let mut diagnostic = Diagnostic::new(
                        "SK403",
                        "Multiline bracket items must be placed one per line",
                        display_path,
                        Span::new(line.no, col_start, line.no, col_end),
                        "warning",
                    );
                    if let Some(replacement) = split_comma_items_preserving_indent(&line.text) {
                        diagnostic = diagnostic.with_fix(Fix {
                            message: "Split multiline bracket items".to_string(),
                            replacement,
                            start_line: line.no,
                            start_column: 1,
                            end_line: line.no,
                            end_column: line.text.chars().count() + 1,
                        });
                    }
                    diagnostics.push(diagnostic);
                }
            }
        }
        update_bracket_stack(&mut stack, &line.code, line.no, line.indent);
    }
}

fn is_pure_closer(trimmed: &str) -> bool {
    matches!(trimmed, ")" | "]" | "}" | ")," | "]," | "},")
}

fn first_extra_item_after_top_level_comma(code: &str) -> Option<(usize, usize)> {
    if is_comprehension_clause_line(code) || contains_comprehension_clause(code) {
        return None;
    }
    let mut depth = 0usize;
    for (idx, ch) in code.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let mut start = idx + ch.len_utf8();
                while start < code.len() {
                    let next = code[start..].chars().next()?;
                    if next.is_whitespace() {
                        start += next.len_utf8();
                    } else {
                        break;
                    }
                }
                if start >= code.len() {
                    return None;
                }
                let first = code[start..].chars().next()?;
                if matches!(first, ')' | ']' | '}') {
                    return None;
                }
                let mut end = start;
                let mut inner_depth = 0usize;
                while end < code.len() {
                    let next = code[end..].chars().next()?;
                    match next {
                        '(' | '[' | '{' => inner_depth += 1,
                        ')' | ']' | '}' if inner_depth > 0 => inner_depth -= 1,
                        ',' if inner_depth == 0 => break,
                        _ => {}
                    }
                    end += next.len_utf8();
                }
                while end > start {
                    let prev_start = previous_char_start(code, end).unwrap_or(start);
                    let prev = code[prev_start..end].chars().next().unwrap_or(' ');
                    if prev.is_whitespace() {
                        end = prev_start;
                    } else {
                        break;
                    }
                }
                return Some((start, end.max(start + first.len_utf8())));
            }
            _ => {}
        }
    }
    None
}

fn is_comprehension_clause_line(code: &str) -> bool {
    let trimmed = code.trim_start();
    trimmed.starts_with("for ") || trimmed.starts_with("async for ") || trimmed.starts_with("if ")
}

fn contains_comprehension_clause(code: &str) -> bool {
    code.contains(" for ") || code.contains(" async for ")
}

fn split_comma_items_preserving_indent(line: &str) -> Option<String> {
    let indent: String = line.chars().take_while(|ch| ch.is_whitespace()).collect();
    let trimmed = line.trim();
    let mut items = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escape = false;

    for (idx, ch) in trimmed.char_indices() {
        if let Some(q) = quote {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let item = trimmed[start..idx].trim();
                if item.is_empty() {
                    return None;
                }
                items.push(item.to_string());
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    let tail = trimmed[start..].trim();
    if !tail.is_empty() {
        items.push(tail.to_string());
    }
    if items.len() < 2 || depth != 0 || quote.is_some() {
        return None;
    }
    Some(
        items
            .into_iter()
            .map(|item| format!("{indent}{item},"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn update_bracket_stack(
    stack: &mut Vec<(char, usize, usize)>,
    code: &str,
    line_no: usize,
    indent: usize,
) {
    for ch in code.chars() {
        match ch {
            '(' | '[' | '{' => stack.push((ch, line_no, indent)),
            ')' | ']' | '}' => {
                stack.pop();
            }
            _ => {}
        }
    }
}

fn run_trailing_comma(display_path: &str, lines: &[LineInfo], diagnostics: &mut Vec<Diagnostic>) {
    let comma_sources: Vec<String> = lines.iter().map(|line| line.code.clone()).collect();

    for (line_idx, line) in lines.iter().enumerate() {
        let comma_source = &comma_sources[line_idx];
        let mut search_start = 0usize;
        while let Some(pos) = comma_source[search_start..].find(',') {
            let byte = search_start + pos;
            if !is_inside_import_block(lines, line_idx, byte)
                && matches!(
                    next_significant_char_after_texts(&comma_sources, line_idx, byte + 1),
                    Some(')' | ']' | '}')
                )
                && !is_single_item_tuple_trailing_comma(&comma_sources, line_idx, byte)
            {
                let col = byte_to_column(&line.text, byte);
                diagnostics.push(
                    Diagnostic::new(
                        "SK404",
                        "Trailing commas are not allowed",
                        display_path,
                        Span::new(line.no, col, line.no, col + 1),
                        "warning",
                    )
                    .with_fix(Fix {
                        message: "Remove trailing comma".to_string(),
                        replacement: String::new(),
                        start_line: line.no,
                        start_column: col,
                        end_line: line.no,
                        end_column: col + 1,
                    }),
                );
            }
            search_start = byte + 1;
        }
    }
}

fn next_significant_char_after_texts(
    lines: &[String],
    line_idx: usize,
    byte_after: usize,
) -> Option<char> {
    for (idx, line) in lines.iter().enumerate().skip(line_idx) {
        let start = if idx == line_idx { byte_after } else { 0 };
        for ch in line.get(start..)?.chars() {
            if !ch.is_whitespace() {
                return Some(ch);
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct BracketFrame {
    opener: char,
    line_idx: usize,
    byte_idx: usize,
}

fn is_inside_import_block(lines: &[LineInfo], line_idx: usize, byte_idx: usize) -> bool {
    let frames = bracket_stack_before(
        &lines
            .iter()
            .map(|line| line.code.clone())
            .collect::<Vec<_>>(),
        line_idx,
        byte_idx,
    );
    frames
        .last()
        .map(|frame| is_import_opener(&lines[frame.line_idx].code, frame.byte_idx))
        .unwrap_or(false)
}

fn is_import_opener(code: &str, open_byte: usize) -> bool {
    let prefix = code[..open_byte].trim_start();
    prefix.starts_with("from ") && prefix.contains(" import")
}

fn is_single_item_tuple_trailing_comma(
    lines: &[String],
    comma_line_idx: usize,
    comma_byte: usize,
) -> bool {
    let Some(frame) = bracket_stack_before(lines, comma_line_idx, comma_byte)
        .last()
        .copied()
    else {
        return false;
    };
    if frame.opener != '(' {
        return false;
    }
    if looks_like_call_or_definition_opener(&lines[frame.line_idx], frame.byte_idx) {
        return false;
    }
    let Some(close) = matching_close_after(lines, comma_line_idx, comma_byte + 1, frame.opener)
    else {
        return false;
    };
    count_top_level_commas_between(lines, frame.line_idx, frame.byte_idx + 1, close.0, close.1) == 1
        && has_non_whitespace_between(
            lines,
            frame.line_idx,
            frame.byte_idx + 1,
            comma_line_idx,
            comma_byte,
        )
}

fn bracket_stack_before(lines: &[String], line_idx: usize, byte_idx: usize) -> Vec<BracketFrame> {
    let mut stack: Vec<BracketFrame> = Vec::new();
    for (idx, line) in lines.iter().enumerate().take(line_idx + 1) {
        let mut end = line.len();
        if idx == line_idx {
            end = byte_idx.min(end);
        }
        for (pos, ch) in line[..end].char_indices() {
            match ch {
                '(' | '[' | '{' => stack.push(BracketFrame {
                    opener: ch,
                    line_idx: idx,
                    byte_idx: pos,
                }),
                ')' | ']' | '}' => {
                    stack.pop();
                }
                _ => {}
            }
        }
    }
    stack
}

fn matching_close_after(
    lines: &[String],
    line_idx: usize,
    byte_after: usize,
    opener: char,
) -> Option<(usize, usize)> {
    let closer = matching_closer(opener)?;
    let mut depth = 0usize;
    for (idx, line) in lines.iter().enumerate().skip(line_idx) {
        let start = if idx == line_idx { byte_after } else { 0 };
        for (rel, ch) in line.get(start..)?.char_indices() {
            let byte = start + rel;
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' if ch == closer && depth == 0 => return Some((idx, byte)),
                ')' | ']' | '}' if depth > 0 => depth -= 1,
                _ => {}
            }
        }
    }
    None
}

fn matching_closer(opener: char) -> Option<char> {
    match opener {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

fn count_top_level_commas_between(
    lines: &[String],
    start_line_idx: usize,
    start_byte: usize,
    end_line_idx: usize,
    end_byte: usize,
) -> usize {
    let mut depth = 0usize;
    let mut count = 0usize;
    for (idx, line) in lines
        .iter()
        .enumerate()
        .take(end_line_idx + 1)
        .skip(start_line_idx)
    {
        let start = if idx == start_line_idx { start_byte } else { 0 };
        let end = if idx == end_line_idx {
            end_byte.min(line.len())
        } else {
            line.len()
        };
        for ch in line[start..end].chars() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' if depth > 0 => depth -= 1,
                ',' if depth == 0 => count += 1,
                _ => {}
            }
        }
    }
    count
}

fn has_non_whitespace_between(
    lines: &[String],
    start_line_idx: usize,
    start_byte: usize,
    end_line_idx: usize,
    end_byte: usize,
) -> bool {
    for (idx, line) in lines
        .iter()
        .enumerate()
        .take(end_line_idx + 1)
        .skip(start_line_idx)
    {
        let start = if idx == start_line_idx { start_byte } else { 0 };
        let end = if idx == end_line_idx {
            end_byte.min(line.len())
        } else {
            line.len()
        };
        if line[start..end].chars().any(|ch| !ch.is_whitespace()) {
            return true;
        }
    }
    false
}

fn looks_like_call_or_definition_opener(line: &str, open_byte: usize) -> bool {
    let prefix = &line[..open_byte];
    let trimmed = prefix.trim_end();
    if trimmed.ends_with("def") || trimmed.ends_with("class") {
        return true;
    }
    let Some(previous) = prefix.chars().next_back() else {
        return false;
    };
    if previous.is_whitespace() {
        return false;
    }
    previous.is_ascii_alphanumeric() || previous == '_' || previous == ']' || previous == ')'
}

fn run_from_import_only(
    display_path: &str,
    source: &str,
    lines: &[LineInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for line in lines {
        let trimmed = line.code.trim_start();
        if !trimmed.starts_with("import ") {
            continue;
        }
        let modules: Vec<&str> = trimmed["import ".len()..]
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        if modules.len() != 1 || modules[0].contains(" as ") || modules[0].contains('.') {
            diagnostics.push(Diagnostic::new(
                "SK502",
                "Imports must use from-import form",
                display_path,
                Span::new(line.no, 1, line.no, line.text.chars().count() + 1),
                "warning",
            ));
            continue;
        }
        let module = modules[0];
        if module == "sys" && sys_import_is_runtime_guard_allowed(source) {
            continue;
        }
        diagnostics.push(Diagnostic::new(
            "SK502",
            "Imports must use from-import form",
            display_path,
            Span::new(line.no, 1, line.no, line.text.chars().count() + 1),
            "warning",
        ));
    }
}

fn sys_import_is_runtime_guard_allowed(source: &str) -> bool {
    source.lines().any(|line| {
        let code = strip_comment_and_strings(line);
        code.contains("sys.platform") || code.contains("sys.version_info")
    })
}

fn run_os_name(display_path: &str, lines: &[LineInfo], diagnostics: &mut Vec<Diagnostic>) {
    for line in lines {
        let mut start = 0usize;
        while let Some(pos) = line.code[start..].find("os.name") {
            let byte = start + pos;
            let col = byte_to_column(&line.text, byte);
            diagnostics.push(Diagnostic::new(
                "SK503",
                "Use sys.platform instead of os.name for platform checks",
                display_path,
                Span::new(line.no, col, line.no, col + "os.name".len()),
                "warning",
            ));
            start = byte + "os.name".len();
        }
    }
}

fn run_sys_platform_import(
    display_path: &str,
    source: &str,
    lines: &[LineInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let has_direct = lines.iter().any(|line| {
        line.code.trim() == "from sys import platform"
            || line.code.trim().starts_with("from sys import platform,")
    });
    if !has_direct {
        return;
    }
    for line in lines {
        let trimmed = line.code.trim_start();
        if trimmed.starts_with("if platform") || trimmed.starts_with("elif platform") {
            let col = line
                .text
                .find("platform")
                .map(|idx| byte_to_column(&line.text, idx))
                .unwrap_or(1);
            diagnostics.push(
                Diagnostic::new(
                    "SK504",
                    "Use import sys and sys.platform so type checkers can narrow platform branches",
                    display_path,
                    Span::new(line.no, col, line.no, col + "platform".len()),
                    "warning",
                )
                .with_fix(Fix {
                    message: "Rewrite platform import to sys.platform".to_string(),
                    replacement: rewrite_platform_import(source),
                    start_line: 1,
                    start_column: 1,
                    end_line: source_line_count(source),
                    end_column: last_line_column(source),
                }),
            );
        }
    }
}

fn rewrite_platform_import(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            if line.trim() == "from sys import platform" {
                "import sys".to_string()
            } else if line.trim_start().starts_with("if platform")
                || line.trim_start().starts_with("elif platform")
            {
                line.replacen("platform", "sys.platform", 1)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn run_definition_order(
    display_path: &str,
    lines: &[LineInfo],
    defs: &[DefInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    run_top_level_definition_order(display_path, lines, defs, diagnostics);
    run_method_definition_order(display_path, lines, defs, diagnostics);
}

fn run_top_level_definition_order(
    display_path: &str,
    lines: &[LineInfo],
    defs: &[DefInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let top: Vec<&DefInfo> = defs.iter().filter(|def| def.indent == 0).collect();
    for def in &top {
        if def.name.starts_with('_') {
            continue;
        }
        for line in lines.iter().take(def.start.saturating_sub(1)) {
            if contains_top_level_definition_reference(&line.code, &def.name) {
                diagnostics.push(Diagnostic::new(
                    "SK505",
                    "Definitions must appear before their first use",
                    display_path,
                    Span::new(line.no, 1, line.no, line.text.chars().count().max(1)),
                    "warning",
                ));
                break;
            }
        }
    }
}

fn contains_top_level_definition_reference(code: &str, name: &str) -> bool {
    code.match_indices(name).any(|(idx, _)| {
        let before = code[..idx].chars().next_back();
        let after = code[idx + name.len()..].chars().next();
        before.is_none_or(|ch| !is_ident_continue(ch)) && matches!(after, Some('(' | '.'))
    })
}

fn run_method_definition_order(
    display_path: &str,
    lines: &[LineInfo],
    defs: &[DefInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut methods_by_class: HashMap<usize, Vec<&DefInfo>> = HashMap::new();
    for def in defs.iter().filter(|def| def.kind == DefKind::Function) {
        if let Some(parent) = def.parent {
            if defs[parent].kind == DefKind::Class {
                methods_by_class.entry(parent).or_default().push(def);
            }
        }
    }
    for methods in methods_by_class.values() {
        let mut positions = HashMap::new();
        for method in methods {
            positions.insert(method.name.as_str(), method.start);
        }
        for method in methods {
            if method.name == "__init__" || method.name == "__post_init__" {
                continue;
            }
            for line in &lines[method.start - 1..method.end.min(lines.len())] {
                for (name, start) in &positions {
                    if *start <= method.start || *name == method.name {
                        continue;
                    }
                    if line.code.contains(&format!("self.{name}(")) {
                        diagnostics.push(Diagnostic::new(
                            "SK505",
                            "Methods must be defined before they are used in the class",
                            display_path,
                            Span::new(line.no, 1, line.no, line.text.chars().count().max(1)),
                            "warning",
                        ));
                        break;
                    }
                }
            }
        }
    }
}

fn run_special_method_order(
    display_path: &str,
    defs: &[DefInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut methods_by_class: HashMap<usize, Vec<&DefInfo>> = HashMap::new();
    for def in defs.iter().filter(|def| def.kind == DefKind::Function) {
        if let Some(parent) = def.parent {
            if defs[parent].kind == DefKind::Class {
                methods_by_class.entry(parent).or_default().push(def);
            }
        }
    }

    for methods in methods_by_class.values_mut() {
        methods.sort_by_key(|method| method.start);
        let mut max_seen_phase = 0usize;
        for method in methods.iter() {
            let phase = special_method_phase(&method.name);
            if phase < max_seen_phase {
                diagnostics.push(Diagnostic::new(
                    "SK509",
                    "__new__, __init__ and __post_init__ must appear before regular methods and in this order",
                    display_path,
                    Span::new(method.start, 1, method.start, 1 + method.name.len()),
                    "warning",
                ));
            }
            max_seen_phase = max_seen_phase.max(phase);
        }
    }
}

fn special_method_phase(name: &str) -> usize {
    match name {
        "__new__" => 0,
        "__init__" => 1,
        "__post_init__" => 2,
        _ => 3,
    }
}

fn run_try_blocks(display_path: &str, lines: &[LineInfo], diagnostics: &mut Vec<Diagnostic>) {
    for line in lines {
        let trimmed = line.code.trim_start();
        if trimmed.starts_with("try:")
            || trimmed.starts_with("except")
            || trimmed.starts_with("finally:")
        {
            diagnostics.push(Diagnostic::new(
                "SK506",
                "try, except and finally blocks are forbidden in hot runtime code",
                display_path,
                Span::new(
                    line.no,
                    line.indent + 1,
                    line.no,
                    line.text.chars().count().max(1),
                ),
                "warning",
            ));
        }
    }
}

fn run_raise_hot_path(
    display_path: &str,
    lines: &[LineInfo],
    defs: &[DefInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let allowed = ["__init__", "__post_init__", "run", "close"];
    for line in lines {
        let trimmed = line.code.trim_start();
        if !trimmed.starts_with("raise") {
            continue;
        }
        let current_def = defs
            .iter()
            .enumerate()
            .filter(|(_, def)| {
                def.start < line.no && line.no <= def.end && def.kind == DefKind::Function
            })
            .max_by_key(|(_, def)| def.indent);
        let allowed_here = current_def.is_some_and(|(_, def)| {
            allowed.contains(&def.name.as_str())
                || def.name.starts_with('_')
                    && private_method_only_used_by_allowed(def, lines, defs, &allowed)
        });
        if allowed_here {
            continue;
        }
        diagnostics.push(Diagnostic::new(
            "SK507",
            "raise is allowed only in __init__, __post_init__, run, close, or private helpers used by them",
            display_path,
            Span::new(line.no, line.indent + 1, line.no, line.indent + 6),
            "warning",
        ));
    }
}

fn private_method_only_used_by_allowed(
    def: &DefInfo,
    lines: &[LineInfo],
    defs: &[DefInfo],
    allowed: &[&str],
) -> bool {
    if !def.name.starts_with('_') {
        return false;
    }
    let mut users = BTreeSet::new();
    let needle = format!("self.{}(", def.name);
    for user in defs
        .iter()
        .filter(|item| item.kind == DefKind::Function && item.start != def.start)
    {
        if lines[user.start - 1..user.end.min(lines.len())]
            .iter()
            .any(|line| line.code.contains(&needle))
        {
            users.insert(user.name.as_str());
        }
    }
    !users.is_empty() && users.iter().all(|name| allowed.contains(name))
}

fn run_future_annotations_import(
    display_path: &str,
    source: &str,
    lines: &[LineInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for line in lines {
        let trimmed = line.code.trim_start();
        let Some(rest) = trimmed.strip_prefix("from __future__ import ") else {
            continue;
        };
        let names = rest
            .trim()
            .trim_matches(['(', ')'])
            .split(',')
            .map(str::trim);
        if !names.clone().any(|name| name == "annotations") {
            continue;
        }

        let column = line
            .text
            .find("annotations")
            .map(|idx| byte_to_column(&line.text, idx))
            .unwrap_or(line.indent + 1);
        let mut diagnostic = Diagnostic::new(
            "SK508",
            "from __future__ import annotations is forbidden",
            display_path,
            Span::new(line.no, column, line.no, column + "annotations".len()),
            "warning",
        );

        if trimmed == "from __future__ import annotations" {
            let (end_line, end_column) = if line.no < source_line_count(source) {
                (line.no + 1, 1)
            } else {
                (line.no, line.text.chars().count() + 1)
            };
            diagnostic = diagnostic.with_fix(Fix {
                message: "Remove future annotations import".to_string(),
                replacement: String::new(),
                start_line: line.no,
                start_column: 1,
                end_line,
                end_column,
            });
        }

        diagnostics.push(diagnostic);
    }
}

fn run_inline_temp_variable(
    display_path: &str,
    lines: &[LineInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for idx in 0..lines.len().saturating_sub(1) {
        let line = &lines[idx];
        let next = &lines[idx + 1];
        if next.indent != line.indent
            || line.code.trim_start().starts_with("for ")
            || line.code.trim_start().starts_with("while ")
        {
            continue;
        }
        let Some((name, expr)) = simple_assignment(&line.code) else {
            continue;
        };
        if !is_safe_inline_expr(expr) {
            continue;
        }
        let next_code = next.code.trim_start();
        if !(next_code.starts_with("return ")
            || next_code.starts_with("yield ")
            || next_code.contains('='))
        {
            continue;
        }
        if count_word_uses(&next.code, name) != 1 {
            continue;
        }
        let mut diagnostic = Diagnostic::new(
            "SK801",
            "Single-use intermediate variables should be inlined in strict mode",
            display_path,
            Span::new(
                line.no,
                line.indent + 1,
                line.no,
                line.text.chars().count() + 1,
            ),
            "warning",
        );
        if next_code.starts_with("return ") || next_code.starts_with("yield ") {
            let replacement = replace_word_once(&next.text, name, expr.trim());
            diagnostic = diagnostic.with_fix(Fix {
                message: "Inline the single-use variable".to_string(),
                replacement,
                start_line: line.no,
                start_column: 1,
                end_line: next.no,
                end_column: next.text.chars().count() + 1,
            });
        }
        diagnostics.push(diagnostic);
    }
}

fn simple_assignment(code: &str) -> Option<(&str, &str)> {
    let trimmed = code.trim_start();
    if trimmed.starts_with("return ")
        || trimmed.starts_with("if ")
        || trimmed.starts_with("for ")
        || trimmed.starts_with("while ")
    {
        return None;
    }
    let (pos, value_start) = if let Some(pos) = trimmed.find(" = ") {
        (pos, pos + 3)
    } else {
        let pos = trimmed.find('=')?;
        (pos, pos + 1)
    };
    if trimmed[..pos].contains(['.', '[', ']', '(', ')', ',']) {
        return None;
    }
    let name = trimmed[..pos].trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        || name.chars().next().unwrap_or('_').is_ascii_digit()
    {
        return None;
    }
    Some((name, trimmed[value_start..].trim()))
}

fn is_safe_inline_expr(expr: &str) -> bool {
    !(expr.contains("await ")
        || expr.contains("yield")
        || expr.contains(" = ")
        || expr.trim().is_empty())
}

fn count_word_uses(text: &str, word: &str) -> usize {
    let mut count = 0usize;
    let mut idx = 0usize;
    while let Some(pos) = text[idx..].find(word) {
        let start = idx + pos;
        let end = start + word.len();
        let before = char_before(text, start);
        let after = text[end..].chars().next();
        if before.is_none_or(|ch| !is_ident_continue(ch))
            && after.is_none_or(|ch| !is_ident_continue(ch))
        {
            count += 1;
        }
        idx = end;
    }
    count
}

fn replace_word_once(text: &str, word: &str, replacement: &str) -> String {
    let mut idx = 0usize;
    while let Some(pos) = text[idx..].find(word) {
        let start = idx + pos;
        let end = start + word.len();
        let before = char_before(text, start);
        let after = text[end..].chars().next();
        if before.is_none_or(|ch| !is_ident_continue(ch))
            && after.is_none_or(|ch| !is_ident_continue(ch))
        {
            let mut out = String::new();
            out.push_str(&text[..start]);
            out.push_str(replacement);
            out.push_str(&text[end..]);
            return out;
        }
        idx = end;
    }
    text.to_string()
}

fn run_return_ternary(display_path: &str, lines: &[LineInfo], diagnostics: &mut Vec<Diagnostic>) {
    for idx in 0..lines.len().saturating_sub(2) {
        let if_line = &lines[idx];
        let return_line = &lines[idx + 1];
        let fallback = &lines[idx + 2];
        let trimmed_if = if_line.code.trim_start();
        if !trimmed_if.starts_with("if ") || !trimmed_if.ends_with(':') {
            continue;
        }
        if return_line.indent <= if_line.indent || fallback.indent != if_line.indent {
            continue;
        }
        let first = return_line.code.trim_start();
        let second = fallback.code.trim_start();
        if !first.starts_with("return ") || !second.starts_with("return ") {
            continue;
        }
        let cond = trimmed_if
            .trim_start_matches("if ")
            .trim_end_matches(':')
            .trim();
        let first_text = return_line.text.trim_start();
        let second_text = fallback.text.trim_start();
        let a = first_text.trim_start_matches("return ").trim();
        let b = second_text.trim_start_matches("return ").trim();
        let replacement = format!(
            "{}return {a} if {cond} else {b}",
            " ".repeat(if_line.indent)
        );
        diagnostics.push(
            Diagnostic::new(
                "SK802",
                "Return branches should be collapsed into a ternary expression in strict mode",
                display_path,
                Span::new(
                    if_line.no,
                    if_line.indent + 1,
                    fallback.no,
                    fallback.text.chars().count() + 1,
                ),
                "warning",
            )
            .with_fix(Fix {
                message: "Collapse returns into a ternary".to_string(),
                replacement,
                start_line: if_line.no,
                start_column: 1,
                end_line: fallback.no,
                end_column: fallback.text.chars().count() + 1,
            }),
        );
    }
}

fn run_loop_comprehension(
    display_path: &str,
    lines: &[LineInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for idx in 0..lines.len().saturating_sub(2) {
        let init = &lines[idx];
        let for_line = &lines[idx + 1];
        let append = &lines[idx + 2];
        let Some((target, literal)) = simple_assignment(&init.code) else {
            continue;
        };
        if literal.trim() != "[]" {
            continue;
        }
        let for_trim = for_line.code.trim_start();
        if !for_trim.starts_with("for ")
            || !for_trim.ends_with(':')
            || for_line.indent != init.indent
        {
            continue;
        }
        let append_trim = append.code.trim_start();
        let needle = format!("{target}.append(");
        if !append_trim.starts_with(&needle) || append.indent <= for_line.indent {
            continue;
        }
        let Some(expr) = append_trim
            .strip_prefix(&needle)
            .and_then(|rest| rest.strip_suffix(')'))
        else {
            continue;
        };
        let for_part = for_trim.trim_end_matches(':');
        let replacement = format!(
            "{}{} = [{expr} {for_part}]",
            " ".repeat(init.indent),
            target
        );
        diagnostics.push(
            Diagnostic::new(
                "SK803",
                "Append-only loops should be list comprehensions in strict mode",
                display_path,
                Span::new(
                    init.no,
                    init.indent + 1,
                    append.no,
                    append.text.chars().count() + 1,
                ),
                "warning",
            )
            .with_fix(Fix {
                message: "Collapse append loop into a list comprehension".to_string(),
                replacement,
                start_line: init.no,
                start_column: 1,
                end_line: append.no,
                end_column: append.text.chars().count() + 1,
            }),
        );
    }
}

fn run_all_tuple(
    display_path: &str,
    source: &str,
    lines: &[LineInfo],
    defs: &[DefInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let public = public_module_symbols(lines, defs);
    if public.is_empty() {
        return;
    }
    let all_line = lines
        .iter()
        .find(|line| line.code.trim_start().starts_with("__all__"));
    if let Some(line) = all_line {
        if !is_all_tuple_assignment(&line.code) {
            diagnostics.push(Diagnostic::new(
                "SK804",
                "__all__ must be declared as a tuple",
                display_path,
                Span::new(line.no, 1, line.no, line.text.chars().count() + 1),
                "warning",
            ));
        }
        return;
    }
    let insert_after = insert_after_imports(lines);
    let mut replacement = source.to_string();
    let insert = format!("\n{}\n", tuple_all_line(&public));
    replacement = insert_text_after_line(&replacement, insert_after, &insert);
    diagnostics.push(
        Diagnostic::new(
            "SK804",
            "Modules with public symbols must define __all__ as a tuple in strict mode",
            display_path,
            Span::new(1, 1, 1, 1),
            "warning",
        )
        .with_fix(Fix {
            message: "Create __all__ tuple template".to_string(),
            replacement,
            start_line: 1,
            start_column: 1,
            end_line: source_line_count(source),
            end_column: last_line_column(source),
        }),
    );
}

fn is_all_tuple_assignment(code: &str) -> bool {
    let trimmed = code.trim_start();
    if !trimmed.starts_with("__all__") {
        return false;
    }
    let Some((_, rhs)) = trimmed.split_once('=') else {
        return false;
    };
    rhs.trim_start().starts_with('(')
}

fn public_module_symbols(lines: &[LineInfo], defs: &[DefInfo]) -> Vec<String> {
    let mut symbols = BTreeSet::new();
    for def in defs
        .iter()
        .filter(|def| def.indent == 0 && !def.name.starts_with('_'))
    {
        symbols.insert(def.name.clone());
    }
    for line in lines {
        if line.indent != 0 {
            continue;
        }
        let trimmed = line.code.trim_start();
        if trimmed.starts_with("from __future__ import ") {
            continue;
        } else if trimmed.starts_with("from ") && trimmed.contains(" import ") {
            for imported in imported_names(trimmed) {
                let alias_or_name = imported.split(" as ").last().unwrap_or(imported).trim();
                if !alias_or_name.starts_with('_') && !alias_or_name.is_empty() {
                    symbols.insert(alias_or_name.to_string());
                }
            }
        } else if let Some((name, _)) = simple_assignment(trimmed) {
            if name
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_uppercase())
                && !name.starts_with('_')
                && name != "__all__"
            {
                symbols.insert(name.to_string());
            }
        }
    }
    symbols.into_iter().collect()
}

fn imported_names(trimmed: &str) -> Vec<&str> {
    trimmed
        .split(" import ")
        .nth(1)
        .unwrap_or("")
        .trim()
        .trim_matches(['(', ')'])
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect()
}

fn tuple_all_line(public: &[String]) -> String {
    let mut items = public
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ");
    if public.len() == 1 {
        items.push(',');
    }
    format!("__all__ = ({items})")
}

fn insert_after_imports(lines: &[LineInfo]) -> usize {
    let mut idx = 0usize;
    let mut last = 0usize;
    while idx < lines.len() {
        let trimmed_text = lines[idx].text.trim_start();
        if trimmed_text.trim().is_empty() || trimmed_text.starts_with('#') {
            last = lines[idx].no;
            idx += 1;
        } else {
            break;
        }
    }
    if idx < lines.len() && lines[idx].text.trim_start().starts_with("\"\"\"") {
        last = lines[idx].no;
        let single_line_docstring = lines[idx]
            .text
            .trim_start()
            .trim_start_matches("\"\"\"")
            .contains("\"\"\"");
        idx += 1;
        if !single_line_docstring {
            while idx < lines.len() {
                last = lines[idx].no;
                if lines[idx].text.contains("\"\"\"") {
                    idx += 1;
                    break;
                }
                idx += 1;
            }
        }
    }
    while idx < lines.len() {
        let trimmed = lines[idx].code.trim_start();
        if trimmed.trim().is_empty() {
            last = lines[idx].no;
            idx += 1;
            continue;
        }
        if trimmed.starts_with("import ") || trimmed.starts_with("from ") {
            let mut depth = update_import_depth(0, &lines[idx].code);
            last = lines[idx].no;
            idx += 1;
            while depth > 0 && idx < lines.len() {
                depth = update_import_depth(depth, &lines[idx].code);
                last = lines[idx].no;
                idx += 1;
            }
            continue;
        }
        break;
    }
    last.max(1)
}

fn update_import_depth(mut depth: usize, code: &str) -> usize {
    for ch in code.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    depth
}

fn insert_text_after_line(source: &str, line_no: usize, text: &str) -> String {
    let mut out = String::new();
    for (idx, line) in source.lines().enumerate() {
        out.push_str(line);
        if idx + 1 == line_no {
            out.push_str(text);
        }
        if idx + 1 < source.lines().count() {
            out.push('\n');
        }
    }
    out
}

fn source_line_count(source: &str) -> usize {
    source.split('\n').count()
}

fn last_line_column(source: &str) -> usize {
    source
        .split('\n')
        .next_back()
        .map(|line| line.chars().count() + 1)
        .unwrap_or(1)
}

fn byte_to_column(text: &str, byte_idx: usize) -> usize {
    text[..byte_idx.min(text.len())].chars().count() + 1
}

#[cfg(test)]
mod tests {
    use crate::config::VscodeConfig;
    use crate::{analyze, AnalysisInput};
    use std::path::PathBuf;

    fn codes(source: &str, strict: bool) -> Vec<String> {
        let mut vscode_config = VscodeConfig::default();
        vscode_config.strict = Some(strict);
        analyze(AnalysisInput {
            path: PathBuf::from("example.py"),
            source: source.to_string(),
            vscode_config,
        })
        .diagnostics
        .into_iter()
        .map(|diag| diag.code)
        .collect()
    }

    #[test]
    fn catches_print_outside_main() {
        let found = codes("def f():\n    print('debug')\n", false);
        assert!(found.contains(&"SK201".to_string()));
    }

    #[test]
    fn allows_print_inside_main_guard() {
        let found = codes("if __name__ == \"__main__\":\n    print('ok')\n", false);
        assert!(!found.contains(&"SK201".to_string()));
    }

    #[test]
    fn catches_assignment_spacing() {
        let found = codes("x=1+2\n", false);
        assert!(found.contains(&"SK401".to_string()));
        assert!(!found.contains(&"SK402".to_string()));
    }

    #[test]
    fn sk401_ignores_augmented_assignments() {
        let source =
            "x += 1\ny -= 1\nz /= 2\nw //= 3\na **= 2\nb %= 2\nc @= m\nd &= 1\ne |= 1\nf ^= 1\n";
        assert!(!codes(source, false).contains(&"SK401".to_string()));
    }

    #[test]
    fn sk403_allows_single_line_call_and_split_multiline_signature() {
        let source = "x = IntVector2(RIGHT_EDGE_POSITION_X, 2)\n\ndef f(\n    self,\n    value: int\n):\n    pass";
        assert!(!codes(source, false).contains(&"SK403".to_string()));
    }

    #[test]
    fn sk403_catches_items_kept_on_one_line_inside_multiline_brackets() {
        let source = "x = IntVector2(\n    RIGHT_EDGE_POSITION_X, 2\n)";
        let mut vscode_config = VscodeConfig::default();
        vscode_config.ignore.push("SK309".to_string());
        let report = analyze(AnalysisInput {
            path: PathBuf::from("example.py"),
            source: source.to_string(),
            vscode_config,
        });
        let diag = report
            .diagnostics
            .into_iter()
            .find(|diag| diag.code == "SK403")
            .expect("SK403 exists");
        assert_eq!((diag.line, diag.column, diag.end_column), (2, 28, 29));
    }

    #[test]
    fn sk403_ignores_generator_unpacking_inside_multiline_call() {
        let source = r#"def f(annotations):
    return tuple(
        (name, annotation)
        for name, annotation in annotations.items()
        if not name.startswith("_")
    )
"#;
        assert!(!codes(source, false).contains(&"SK403".to_string()));
    }

    #[test]
    fn sk404_only_catches_commas_without_following_elements() {
        let ok = "def draw_param(\n    self,\n    param_name: str,\n    val: float\n):\n    pass";
        assert!(!codes(ok, false).contains(&"SK404".to_string()));

        let bad = "x = (a, b, c,)";
        let mut vscode_config = VscodeConfig::default();
        vscode_config.ignore.push("SK309".to_string());
        let report = analyze(AnalysisInput {
            path: PathBuf::from("example.py"),
            source: bad.to_string(),
            vscode_config,
        });
        let diag = report
            .diagnostics
            .into_iter()
            .find(|diag| diag.code == "SK404")
            .expect("SK404 exists");
        assert_eq!((diag.line, diag.column, diag.end_column), (1, 13, 14));
    }

    #[test]
    fn sk404_ignores_parenthesized_import_blocks() {
        let source = "from module import (\n    one,\n    two,\n)\n";
        assert!(!codes(source, false).contains(&"SK404".to_string()));
    }

    #[test]
    fn sk404_preserves_string_literals_when_finding_next_element() {
        let source = "if hasattr(data, \"_asdict\"):\n    pass\n";
        assert!(!codes(source, false).contains(&"SK404".to_string()));
    }

    #[test]
    fn syntax_rules_ignore_docstrings_and_comments() {
        let source = r#"def f():
    """
    IntVector2(
        RIGHT_EDGE_POSITION_X, 2
    )
    x = (a, b, c,)
    """
    # IntVector2(
    #     RIGHT_EDGE_POSITION_X, 2
    # )
    pass
"#;
        let found = codes(source, false);
        assert!(!found.contains(&"SK403".to_string()));
        assert!(!found.contains(&"SK404".to_string()));
    }

    #[test]
    fn strict_catches_return_ternary_and_all() {
        let found = codes(
            "def f(a, b, c):\n    if a > 0:\n        return b\n    return c\n",
            true,
        );
        assert!(found.contains(&"SK802".to_string()));
        assert!(found.contains(&"SK804".to_string()));
    }

    #[test]
    fn strict_catches_append_loop() {
        let found = codes("def f(items):\n    out = []\n    for item in items:\n        out.append(item.value)\n    return out\n", true);
        assert!(found.contains(&"SK803".to_string()));
    }

    #[test]
    fn sk502_allows_import_sys_for_platform_or_version_info_guards() {
        let platform = "import sys

if sys.platform == \"win32\":
    pass
";
        assert!(!codes(platform, false).contains(&"SK502".to_string()));

        let version = "import sys

if sys.version_info >= (3, 12):
    pass
";
        assert!(!codes(version, false).contains(&"SK502".to_string()));
    }

    #[test]
    fn sk502_reports_import_sys_without_platform_or_version_info() {
        let found = codes(
            "import sys

print(sys.executable)
",
            false,
        );
        assert!(found.contains(&"SK502".to_string()));
    }

    #[test]
    fn sk502_ignores_sys_mentions_in_comments_and_strings() {
        let found = codes(
            "import sys
# sys.platform
text = \"sys.version_info\"
print(sys.executable)
",
            false,
        );
        assert!(found.contains(&"SK502".to_string()));
    }

    #[test]
    fn catches_future_annotations_import() {
        let found = codes("from __future__ import annotations\n", false);
        assert!(found.contains(&"SK508".to_string()));
    }

    #[test]
    fn future_annotations_are_not_public_all_symbols() {
        let mut vscode_config = VscodeConfig::default();
        vscode_config.strict = Some(true);
        let report = analyze(AnalysisInput {
            path: PathBuf::from("example.py"),
            source: "from __future__ import annotations\n\nclass Public:\n    pass\n".to_string(),
            vscode_config,
        });
        let all_fix = report
            .diagnostics
            .into_iter()
            .find(|diag| diag.code == "SK804")
            .and_then(|diag| diag.fix)
            .expect("SK804 fix exists");
        let all_line = all_fix
            .replacement
            .lines()
            .find(|line| line.starts_with("__all__"))
            .expect("__all__ line exists");
        assert!(!all_line.contains("annotations"));
    }
    #[test]
    fn sk404_allows_single_item_tuple_but_reports_multi_item_tuple_trailing_comma() {
        for source in [
            "x = (value,)\n",
            "for module in (module,):\n    pass\n",
            "return_value = call((module,))\n",
        ] {
            assert!(
                !codes(source, false).contains(&"SK404".to_string()),
                "source should be accepted: {source}"
            );
        }

        let found = codes("x = (a, b,)\n", false);
        assert!(found.contains(&"SK404".to_string()));
    }

    #[test]
    fn sk404_reports_call_trailing_comma_even_with_single_argument() {
        let found = codes("result = call(value,)\n", false);
        assert!(found.contains(&"SK404".to_string()));
    }

    #[test]
    fn sk505_uses_identifier_boundaries_for_top_level_names() {
        let found = codes(
            "class GnssNavData(GlobalNavData):\n    pass\n\nclass NavData:\n    pass\n",
            false,
        );
        assert!(!found.contains(&"SK505".to_string()));
    }

    #[test]
    fn sk509_reports_init_or_post_init_after_regular_method_separately_from_sk505() {
        let found = codes("class Box:\n    def helper(self):\n        pass\n\n    def __init__(self):\n        pass\n\n    def __post_init__(self):\n        pass\n", false);
        assert!(found.contains(&"SK509".to_string()));
        assert!(!found.contains(&"SK505".to_string()));
    }

    #[test]
    fn sk509_allows_new_init_then_post_init_before_regular_methods() {
        let found = codes(
            "class Box:
    def __new__(cls):
        return super().__new__(cls)

    def __init__(self):
        pass

    def __post_init__(self):
        pass

    def helper(self):
        pass
",
            false,
        );
        assert!(!found.contains(&"SK509".to_string()));
    }

    #[test]
    fn sk509_reports_new_after_init_or_regular_method() {
        let after_init = codes(
            "class Box:
    def __init__(self):
        pass

    def __new__(cls):
        return super().__new__(cls)
",
            false,
        );
        assert!(after_init.contains(&"SK509".to_string()));

        let after_regular = codes(
            "class Box:
    def helper(self):
        pass

    def __new__(cls):
        return super().__new__(cls)
",
            false,
        );
        assert!(after_regular.contains(&"SK509".to_string()));
    }

    #[test]
    fn annotated_all_tuple_is_accepted_and_not_autofixed() {
        let mut vscode_config = VscodeConfig::default();
        vscode_config.strict = Some(true);
        let report = analyze(AnalysisInput {
            path: PathBuf::from("example.py"),
            source: "from typing import Final\n\n__all__: Final[tuple[str, ...]] = (\n    \"Public\"\n)\n\nclass Public:\n    pass\n".to_string(),
            vscode_config,
        });
        assert!(!report.diagnostics.iter().any(|diag| diag.code == "SK804"));
    }

    #[test]
    fn sk502_and_sk503_do_not_offer_unsafe_fixes() {
        let mut vscode_config = VscodeConfig::default();
        vscode_config.strict = Some(false);
        let report = analyze(AnalysisInput {
            path: PathBuf::from("example.py"),
            source: "import os\nimport __main__\n\nif os.name == \"nt\":\n    print(__main__.__file__)\n".to_string(),
            vscode_config,
        });
        for code in ["SK502", "SK503"] {
            let diag = report
                .diagnostics
                .iter()
                .find(|diag| diag.code == code)
                .expect("diagnostic exists");
            assert!(diag.fix.is_none(), "{code} must not have an unsafe fix");
        }
    }

    #[test]
    fn strings_continued_with_backslash_are_masked() {
        let found = codes("value = f\"prefix \\\n    name={name} \\\n\"\n", false);
        assert!(!found.contains(&"SK401".to_string()));
    }
}
