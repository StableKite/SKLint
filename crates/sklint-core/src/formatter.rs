use crate::analyzer::{analyze, AnalysisInput};
use crate::config::VscodeConfig;
use crate::diagnostic::Fix;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatReport {
    pub source: String,
    pub applied: usize,
}

pub fn format_source(path: PathBuf, source: String, vscode_config: VscodeConfig) -> FormatReport {
    let mut current = source;
    let mut total_applied = 0usize;

    for _ in 0..6 {
        let order_report = apply_ordering_fixes(&current);
        if order_report.applied > 0 {
            current = order_report.source;
            total_applied += order_report.applied;
        }

        let report = analyze(AnalysisInput {
            path: path.clone(),
            source: current.clone(),
            vscode_config: vscode_config.clone(),
        });
        let mut fixes: Vec<Fix> = report
            .diagnostics
            .into_iter()
            .filter_map(|diag| diag.fix)
            .collect();
        if fixes.is_empty() {
            break;
        }
        fixes.sort_by(|a, b| {
            (b.start_line, b.start_column, b.end_line, b.end_column).cmp(&(
                a.start_line,
                a.start_column,
                a.end_line,
                a.end_column,
            ))
        });

        let before = current.clone();
        let mut applied_this_round = 0usize;
        let mut applied_ranges: Vec<(usize, usize, usize, usize)> = Vec::new();
        for fix in fixes {
            let range = (
                fix.start_line,
                fix.start_column,
                fix.end_line,
                fix.end_column,
            );
            if applied_ranges.iter().any(|old| ranges_overlap(*old, range)) {
                continue;
            }
            if apply_fix(&mut current, &fix) {
                applied_ranges.push(range);
                applied_this_round += 1;
            }
        }
        total_applied += applied_this_round;
        if applied_this_round == 0 || current == before {
            break;
        }
    }

    FormatReport {
        source: current,
        applied: total_applied,
    }
}

fn ranges_overlap(a: (usize, usize, usize, usize), b: (usize, usize, usize, usize)) -> bool {
    let (a_start_line, a_start_col, a_end_line, a_end_col) = a;
    let (b_start_line, b_start_col, b_end_line, b_end_col) = b;
    let a_start = (a_start_line, a_start_col);
    let a_end = (a_end_line, a_end_col);
    let b_start = (b_start_line, b_start_col);
    let b_end = (b_end_line, b_end_col);
    a_start < b_end && b_start < a_end
}

fn apply_fix(source: &mut String, fix: &Fix) -> bool {
    let Some(start) = line_col_to_byte(source, fix.start_line, fix.start_column) else {
        return false;
    };
    let Some(end) = line_col_to_byte(source, fix.end_line, fix.end_column) else {
        return false;
    };
    if start > end || end > source.len() {
        return false;
    }
    source.replace_range(start..end, &fix.replacement);
    true
}

fn line_col_to_byte(source: &str, line: usize, column: usize) -> Option<usize> {
    if line == 0 || column == 0 {
        return None;
    }
    let mut current_line = 1usize;
    let mut line_start = 0usize;
    for (idx, ch) in source.char_indices() {
        if current_line == line {
            break;
        }
        if ch == '\n' {
            current_line += 1;
            line_start = idx + 1;
        }
    }
    if current_line != line {
        if line == current_line + 1 && column == 1 {
            return Some(source.len());
        }
        return None;
    }

    let line_text = &source[line_start..];
    let line_end_rel = line_text.find('\n').unwrap_or(line_text.len());
    let logical_line = &line_text[..line_end_rel];
    if column == logical_line.chars().count() + 1 {
        return Some(line_start + logical_line.len());
    }

    for (char_idx, (byte_idx, _)) in logical_line.char_indices().enumerate() {
        if char_idx + 1 == column {
            return Some(line_start + byte_idx);
        }
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormatDefKind {
    Class,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FormatLine {
    no: usize,
    text: String,
    code: String,
    indent: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FormatDef {
    kind: FormatDefKind,
    name: String,
    start: usize,
    group_start: usize,
    end: usize,
    indent: usize,
    parent: Option<usize>,
}

fn apply_ordering_fixes(source: &str) -> FormatReport {
    let mut current = source.to_string();
    let mut applied = 0usize;

    for _ in 0..12 {
        let before = current.clone();
        if reorder_special_methods_once(&mut current)
            || reorder_method_dependencies_once(&mut current)
            || reorder_top_level_definitions_once(&mut current)
        {
            applied += 1;
        }
        if current == before {
            break;
        }
    }

    FormatReport {
        source: current,
        applied,
    }
}

fn reorder_special_methods_once(source: &mut String) -> bool {
    let lines = format_lines(source);
    let defs = parse_format_defs(&lines);

    for class_idx in defs
        .iter()
        .enumerate()
        .filter(|(_, def)| def.kind == FormatDefKind::Class)
        .map(|(idx, _)| idx)
    {
        let mut methods: Vec<&FormatDef> = defs
            .iter()
            .filter(|def| def.kind == FormatDefKind::Function && def.parent == Some(class_idx))
            .collect();
        methods.sort_by_key(|method| method.start);

        for (idx, method) in methods.iter().enumerate() {
            let phase = special_method_format_phase(&method.name);
            let Some(destination) = methods[..idx]
                .iter()
                .find(|candidate| special_method_format_phase(&candidate.name) > phase)
            else {
                continue;
            };
            if move_line_range_before(
                source,
                method.group_start,
                method.end,
                destination.group_start,
            ) {
                return true;
            }
        }
    }

    false
}

fn reorder_method_dependencies_once(source: &mut String) -> bool {
    let lines = format_lines(source);
    let defs = parse_format_defs(&lines);

    for class_idx in defs
        .iter()
        .enumerate()
        .filter(|(_, def)| def.kind == FormatDefKind::Class)
        .map(|(idx, _)| idx)
    {
        let mut methods: Vec<&FormatDef> = defs
            .iter()
            .filter(|def| def.kind == FormatDefKind::Function && def.parent == Some(class_idx))
            .collect();
        methods.sort_by_key(|method| method.start);
        let positions: HashMap<&str, &FormatDef> = methods
            .iter()
            .map(|method| (method.name.as_str(), *method))
            .collect();

        for method in &methods {
            if method.name == "__init__" || method.name == "__post_init__" {
                continue;
            }
            let body_end = method.end.min(lines.len());
            for line in &lines[method.start.saturating_sub(1)..body_end] {
                for (name, target) in &positions {
                    if target.start <= method.start || *name == method.name {
                        continue;
                    }
                    if contains_self_method_call(&line.code, name)
                        && move_line_range_before(
                            source,
                            target.group_start,
                            target.end,
                            method.group_start,
                        )
                    {
                        return true;
                    }
                }
            }
        }
    }

    false
}

fn reorder_top_level_definitions_once(source: &mut String) -> bool {
    let lines = format_lines(source);
    let defs = parse_format_defs(&lines);
    let mut top: Vec<&FormatDef> = defs.iter().filter(|def| def.indent == 0).collect();
    top.sort_by_key(|def| def.start);

    for def in top {
        if def.name.starts_with('_') {
            continue;
        }
        let Some(reference_line) = lines
            .iter()
            .take(def.start.saturating_sub(1))
            .find(|line| contains_top_level_format_reference(&line.code, &def.name))
        else {
            continue;
        };
        let destination =
            enclosing_top_level_block_start(reference_line.no, &defs).unwrap_or(reference_line.no);
        if destination >= def.group_start && destination <= def.end {
            continue;
        }
        if move_line_range_before(source, def.group_start, def.end, destination) {
            return true;
        }
    }

    false
}

fn special_method_format_phase(name: &str) -> usize {
    match name {
        "__new__" => 0,
        "__init__" => 1,
        "__post_init__" => 2,
        _ => 3,
    }
}

fn contains_self_method_call(code: &str, name: &str) -> bool {
    code.match_indices(name).any(|(idx, _)| {
        let before = &code[..idx];
        let after = code[idx + name.len()..].chars().next();
        before.ends_with("self.") && matches!(after, Some('('))
    })
}

fn contains_top_level_format_reference(code: &str, name: &str) -> bool {
    code.match_indices(name).any(|(idx, _)| {
        let before = code[..idx].chars().next_back();
        let after = code[idx + name.len()..].chars().next();
        before.is_none_or(|ch| !is_ident_continue(ch)) && matches!(after, Some('(' | '.'))
    })
}

fn enclosing_top_level_block_start(line_no: usize, defs: &[FormatDef]) -> Option<usize> {
    defs.iter()
        .filter(|def| def.indent == 0 && def.group_start <= line_no && line_no <= def.end)
        .map(|def| def.group_start)
        .min()
}

fn move_line_range_before(
    source: &mut String,
    start: usize,
    end: usize,
    destination: usize,
) -> bool {
    if start == 0
        || end < start
        || destination == 0
        || (start <= destination && destination <= end + 1)
    {
        return false;
    }

    let mut lines = split_preserving_logical_lines(source);
    if end > lines.len() || destination > lines.len() + 1 {
        return false;
    }

    let start_idx = start - 1;
    let end_idx = end;
    let destination_idx = destination - 1;
    let block: Vec<String> = lines.drain(start_idx..end_idx).collect();
    let insert_idx = if destination_idx > start_idx {
        destination_idx.saturating_sub(block.len())
    } else {
        destination_idx
    };
    for (offset, line) in block.into_iter().enumerate() {
        lines.insert(insert_idx + offset, line);
    }

    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    *source = join_logical_lines(&lines, false);
    true
}

fn split_preserving_logical_lines(source: &str) -> Vec<String> {
    let mut lines: Vec<String> = source
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect();
    if source.ends_with('\n') {
        lines.pop();
    }
    lines
}

fn join_logical_lines(lines: &[String], had_final_newline: bool) -> String {
    let mut out = lines.join("\n");
    if had_final_newline {
        out.push('\n');
    }
    out
}

fn format_lines(source: &str) -> Vec<FormatLine> {
    let raw_lines = split_preserving_logical_lines(source);
    let code_lines = mask_format_non_code(source);
    raw_lines
        .into_iter()
        .enumerate()
        .map(|(idx, text)| {
            let code = code_lines.get(idx).cloned().unwrap_or_default();
            let indent = indent_width(&text);
            FormatLine {
                no: idx + 1,
                text,
                code,
                indent,
            }
        })
        .collect()
}

fn parse_format_defs(lines: &[FormatLine]) -> Vec<FormatDef> {
    let mut defs = Vec::new();
    for line in lines {
        let trimmed = line.code.trim_start();
        let kind_name = if trimmed.starts_with("class ") {
            parse_format_name_after_keyword(trimmed, "class ")
                .map(|name| (FormatDefKind::Class, name))
        } else if trimmed.starts_with("def ") {
            parse_format_name_after_keyword(trimmed, "def ")
                .map(|name| (FormatDefKind::Function, name))
        } else if trimmed.starts_with("async def ") {
            parse_format_name_after_keyword(trimmed, "async def ")
                .map(|name| (FormatDefKind::Function, name))
        } else {
            None
        };
        let Some((kind, name)) = kind_name else {
            continue;
        };
        let start = line.no;
        let group_start = format_decorator_group_start(lines, start);
        let end = format_block_end(lines, start, line.indent);
        defs.push(FormatDef {
            kind,
            name,
            start,
            group_start,
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

fn parse_format_name_after_keyword(trimmed: &str, keyword: &str) -> Option<String> {
    let rest = trimmed.strip_prefix(keyword)?.trim_start();
    let name: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn format_decorator_group_start(lines: &[FormatLine], start: usize) -> usize {
    let mut group_start = start;
    while group_start > 1 {
        let previous = &lines[group_start - 2];
        let trimmed = previous.code.trim_start();
        if trimmed.starts_with('@') || trimmed.starts_with('#') && !previous.text.trim().is_empty()
        {
            group_start -= 1;
            continue;
        }
        break;
    }
    group_start
}

fn format_block_end(lines: &[FormatLine], start: usize, indent: usize) -> usize {
    let header_end = format_header_end_line(lines, start);
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
    last_nonblank_format_line(lines, start, end).unwrap_or(start)
}

fn format_header_end_line(lines: &[FormatLine], start: usize) -> usize {
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

fn last_nonblank_format_line(lines: &[FormatLine], start: usize, end: usize) -> Option<usize> {
    if end < start {
        return None;
    }
    (start..=end)
        .rev()
        .find(|line_no| !lines[*line_no - 1].text.trim().is_empty())
}

fn mask_format_non_code(source: &str) -> Vec<String> {
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

fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .count()
}

fn is_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_trailing_whitespace() {
        let report = format_source(
            PathBuf::from("example.py"),
            "x = 1  \n".to_string(),
            VscodeConfig::default(),
        );
        assert_eq!(report.source, "x = 1");
        assert!(report.applied > 0);
    }

    #[test]
    fn reorders_function_sections() {
        let source = "def f(x: int) -> int:\n    \"\"\"\n    Описание\n\n    Returns:\n        int: значение\n    Args:\n        x (int): значение\n    \"\"\"\n    return x\n";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );
        assert!(report.source.find("Args:").unwrap() < report.source.find("Returns:").unwrap());
    }

    #[test]
    fn rewrites_dataclass_attributes() {
        let source = "from dataclasses import dataclass\n\n@dataclass\nclass Base:\n    x: int\n\n@dataclass\nclass Child(Base):\n    \"\"\"\n    Описание\n\n    Attributes:\n        y (int): значение\n    \"\"\"\n    y: str\n";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );
        assert!(report.source.contains("x (int): TODO: описание"));
        assert!(report.source.contains("y (str): значение"));
        assert!(report.source.find("x (int)").unwrap() < report.source.find("y (str)").unwrap());
    }

    #[test]
    fn reorders_special_methods() {
        let source = "class Box:\n    def helper(self):\n        return 1\n\n    def __post_init__(self):\n        pass\n\n    def __new__(cls):\n        return super().__new__(cls)\n\n    def __init__(self):\n        pass\n";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );
        assert!(
            report.source.find("def __new__").unwrap()
                < report.source.find("def __init__").unwrap()
        );
        assert!(
            report.source.find("def __init__").unwrap()
                < report.source.find("def __post_init__").unwrap()
        );
        assert!(
            report.source.find("def __post_init__").unwrap()
                < report.source.find("def helper").unwrap()
        );
    }

    #[test]
    fn reorders_method_dependencies() {
        let source = "class Box:\n    def public(self):\n        return self._helper()\n\n    def _helper(self):\n        return 1\n";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );
        assert!(
            report.source.find("def _helper").unwrap() < report.source.find("def public").unwrap()
        );
    }

    #[test]
    fn reorders_top_level_definitions() {
        let source = "def build():\n    return Box()\n\nclass Box:\n    pass\n";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );
        assert!(
            report.source.find("class Box").unwrap() < report.source.find("def build").unwrap()
        );
    }
}
