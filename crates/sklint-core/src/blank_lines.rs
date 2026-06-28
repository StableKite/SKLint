use crate::config::EffectiveConfig;
use crate::diagnostic::{Diagnostic, Fix, Span};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    Class,
    Function,
    Main,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    kind: NodeKind,
    name: String,
    start: usize,
    group_start: usize,
    body_end: usize,
    end_nonblank: usize,
    indent: usize,
    parent: Option<usize>,
}

pub fn run_blank_line_rules(
    path: &Path,
    source: &str,
    config: &EffectiveConfig,
) -> Vec<Diagnostic> {
    let display_path = path.display().to_string();
    let lines = logical_lines(source);
    let nodes = parse_nodes(&lines);
    let mut diagnostics = Vec::new();

    if config.is_enabled("SK309") {
        run_final_newline_rule(&display_path, source, &lines, &mut diagnostics);
    }

    if config.is_enabled("SK306")
        || config.is_enabled("SK307")
        || config.is_enabled("SK310")
        || config.is_enabled("SK312")
        || config.is_enabled("SK314")
    {
        run_top_level_spacing_rules(
            &display_path,
            source,
            &lines,
            &nodes,
            config,
            &mut diagnostics,
        );
    }

    if config.is_enabled("SK301") || config.is_enabled("SK303") || config.is_enabled("SK311") {
        run_class_spacing_rules(&display_path, &lines, &nodes, config, &mut diagnostics);
    }

    if config.is_enabled("SK302") || config.is_enabled("SK305") || config.is_enabled("SK308") {
        run_body_blank_rules(&display_path, &lines, &nodes, config, &mut diagnostics);
    }

    if config.is_enabled("SK313") {
        run_stub_ellipsis_docstring_spacing(&display_path, &lines, &nodes, &mut diagnostics);
    }

    if config.is_enabled("SK315") {
        run_stub_docstring_ellipsis_spacing(&display_path, &lines, &nodes, &mut diagnostics);
    }

    diagnostics
}

fn logical_lines(source: &str) -> Vec<String> {
    let mut lines: Vec<String> = source
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect();
    if source.ends_with('\n') {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn parse_nodes(lines: &[String]) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut triple_quote: Option<&str> = None;
    for (idx, line) in lines.iter().enumerate() {
        if let Some(quote) = triple_quote {
            if line.contains(quote) {
                triple_quote = None;
            }
            continue;
        }
        if let Some(quote) = opening_triple_quote(line) {
            let start = line.find(quote).unwrap_or(0) + 3;
            if !line[start..].contains(quote) {
                triple_quote = Some(quote);
            }
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@') {
            continue;
        }
        let kind_name = if trimmed.starts_with("class ") {
            parse_name_after_keyword(trimmed, "class ").map(|name| (NodeKind::Class, name))
        } else if trimmed.starts_with("def ") {
            parse_name_after_keyword(trimmed, "def ").map(|name| (NodeKind::Function, name))
        } else if trimmed.starts_with("async def ") {
            parse_name_after_keyword(trimmed, "async def ").map(|name| (NodeKind::Function, name))
        } else if is_main_block(trimmed) {
            Some((NodeKind::Main, "__main__".to_string()))
        } else {
            None
        };
        let Some((kind, name)) = kind_name else {
            continue;
        };
        let start = idx + 1;
        let indent = indent_width(line);
        let group_start = decorator_group_start(lines, start);
        let body_end = block_end(lines, start, indent);
        let end_nonblank = last_nonblank_in_range(lines, start, body_end).unwrap_or(start);
        nodes.push(Node {
            kind,
            name,
            start,
            group_start,
            body_end,
            end_nonblank,
            indent,
            parent: None,
        });
    }

    for idx in 0..nodes.len() {
        let start = nodes[idx].start;
        let indent = nodes[idx].indent;
        nodes[idx].parent = (0..nodes.len())
            .filter(|candidate| *candidate != idx)
            .filter(|candidate| {
                nodes[*candidate].start < start && start <= nodes[*candidate].body_end
            })
            .filter(|candidate| nodes[*candidate].indent < indent)
            .max_by_key(|candidate| nodes[*candidate].indent);
    }

    nodes
}

fn opening_triple_quote(line: &str) -> Option<&'static str> {
    let double = line.find("\"\"\"");
    let single = line.find("'''");
    match (double, single) {
        (Some(d), Some(s)) if d < s => Some("\"\"\""),
        (Some(_), Some(_)) => Some("'''"),
        (Some(_), None) => Some("\"\"\""),
        (None, Some(_)) => Some("'''"),
        (None, None) => None,
    }
}

fn parse_name_after_keyword(trimmed: &str, keyword: &str) -> Option<String> {
    let rest = trimmed.strip_prefix(keyword)?.trim_start();
    let name: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn is_main_block(trimmed: &str) -> bool {
    let compact: String = trimmed.chars().filter(|ch| !ch.is_whitespace()).collect();
    compact == "if__name__==\"__main__\":" || compact == "if__name__=='__main__':"
}

fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum()
}

fn decorator_group_start(lines: &[String], start: usize) -> usize {
    let indent = indent_width(&lines[start - 1]);
    let mut line_no = start;
    while line_no > 1 {
        let prev = &lines[line_no - 2];
        if prev.trim_start().starts_with('@') && indent_width(prev) == indent {
            line_no -= 1;
        } else {
            break;
        }
    }
    line_no
}

fn block_end(lines: &[String], start: usize, indent: usize) -> usize {
    let header_end = header_end_line(lines, start);
    let mut end = lines.len();
    for line_no in header_end + 1..=lines.len() {
        let line = &lines[line_no - 1];
        if line.trim().is_empty() {
            continue;
        }
        if indent_width(line) <= indent {
            end = line_no - 1;
            break;
        }
    }
    end
}

fn header_end_line(lines: &[String], start: usize) -> usize {
    let mut depth = 0usize;
    for line_no in start..=lines.len() {
        let code = strip_comment_and_strings(&lines[line_no - 1]);
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

fn strip_comment_and_strings(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;
    let mut escape = false;
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == q {
                quote = None;
            }
            out.push(' ');
            continue;
        }
        if ch == '#' {
            out.push(' ');
            out.extend(chars.map(|_| ' '));
            break;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            out.push(' ');
            continue;
        }
        out.push(ch);
    }
    out
}

fn last_nonblank_in_range(lines: &[String], start: usize, end: usize) -> Option<usize> {
    if end < start {
        return None;
    }
    (start..=end)
        .rev()
        .find(|line_no| !lines[*line_no - 1].trim().is_empty())
}

fn first_nonblank_in_range(lines: &[String], start: usize, end: usize) -> Option<usize> {
    if end < start {
        return None;
    }
    (start..=end).find(|line_no| !lines[*line_no - 1].trim().is_empty())
}

fn run_final_newline_rule(
    display_path: &str,
    source: &str,
    lines: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !source.ends_with(['\n', '\r']) {
        return;
    }
    let start_line = lines.len();
    let start_column = lines
        .last()
        .map(|line| line.chars().count() + 1)
        .unwrap_or(1);
    let trailing_newlines = source
        .chars()
        .rev()
        .take_while(|ch| *ch == '\n' || *ch == '\r')
        .count();
    diagnostics.push(
        Diagnostic::new(
            "SK309",
            "Files must not end with a newline",
            display_path,
            Span::new(start_line, start_column, start_line + trailing_newlines, 1),
            "warning",
        )
        .with_fix(Fix {
            message: "Remove the final newline".to_string(),
            replacement: String::new(),
            start_line,
            start_column,
            end_line: start_line + trailing_newlines,
            end_column: 1,
        }),
    );
}

fn run_top_level_spacing_rules(
    display_path: &str,
    source: &str,
    lines: &[String],
    nodes: &[Node],
    config: &EffectiveConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let top_level: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.indent == 0)
        .filter(|(_, node)| {
            matches!(
                node.kind,
                NodeKind::Class | NodeKind::Function | NodeKind::Main
            )
        })
        .map(|(idx, _)| idx)
        .collect();

    for pair in top_level.windows(2) {
        let left_idx = pair[0];
        let right_idx = pair[1];
        let left = &nodes[left_idx];
        let right = &nodes[right_idx];
        let actual = blank_count_between(left.end_nonblank, right.group_start);

        if has_intervening_top_level_statement(
            lines,
            left.end_nonblank + 1,
            right.group_start.saturating_sub(1),
        ) {
            continue;
        }

        if right.kind == NodeKind::Main {
            if config.is_enabled("SK307") {
                push_blank_diagnostic(
                    diagnostics,
                    display_path,
                    lines,
                    left.end_nonblank,
                    right.group_start,
                    actual,
                    blank_rule(
                        "SK307",
                        "The __main__ block must be preceded by exactly 3 blank lines",
                        "Set 3 blank lines before the __main__ block",
                        3,
                    ),
                );
            }
            continue;
        }

        if left.kind == NodeKind::Class
            && right.kind == NodeKind::Class
            && is_stub_class(left_idx, nodes, lines)
            && is_stub_class(right_idx, nodes, lines)
        {
            if config.is_enabled("SK312") {
                push_blank_diagnostic(
                    diagnostics,
                    display_path,
                    lines,
                    left.end_nonblank,
                    right.group_start,
                    actual,
                    blank_rule(
                        "SK312",
                        "Stub classes must be separated by exactly 2 blank lines",
                        "Set 2 blank lines between stub classes",
                        2,
                    ),
                );
            }
            continue;
        }

        if is_private_helper_for_next(left_idx, right_idx, nodes, source) {
            if config.is_enabled("SK310") {
                push_blank_diagnostic(
                    diagnostics,
                    display_path,
                    lines,
                    left.end_nonblank,
                    right.group_start,
                    actual,
                    blank_rule("SK310", "A private helper used only by the following object must be separated by exactly 2 blank lines", "Set 2 blank lines after the private helper", 2),
                );
            }
            continue;
        }

        if is_public_standalone(left) && is_public_standalone(right) && config.is_enabled("SK306") {
            push_blank_diagnostic(
                    diagnostics,
                    display_path,
                    lines,
                    left.end_nonblank,
                    right.group_start,
                    actual,
                    blank_rule("SK306", "Standalone public functions and classes must be separated by exactly 3 blank lines", "Set 3 blank lines between standalone public objects", 3),
                );
        }
    }

    if config.is_enabled("SK314") {
        run_type_checking_stub_class_spacing(display_path, lines, nodes, diagnostics);
    }
}

fn run_type_checking_stub_class_spacing(
    display_path: &str,
    lines: &[String],
    nodes: &[Node],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (block_start, block_end, block_indent) in type_checking_blocks(lines) {
        let mut classes: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.kind == NodeKind::Class)
            .filter(|(_, node)| node.start > block_start && node.start <= block_end)
            .filter(|(_, node)| node.indent == block_indent + 4)
            .filter(|(idx, _)| is_argumentless_stub_class(*idx, nodes, lines))
            .map(|(idx, _)| idx)
            .collect();
        classes.sort_by_key(|idx| nodes[*idx].start);
        for pair in classes.windows(2) {
            let left = &nodes[pair[0]];
            let right = &nodes[pair[1]];
            let actual = blank_count_between(left.end_nonblank, right.group_start);
            push_blank_diagnostic(
                diagnostics,
                display_path,
                lines,
                left.end_nonblank,
                right.group_start,
                actual,
                blank_rule(
                    "SK314",
                    "Argumentless TYPE_CHECKING stub classes must be separated by exactly 1 blank line",
                    "Set 1 blank line between TYPE_CHECKING stub classes",
                    1,
                ),
            );
        }
    }
}

fn type_checking_blocks(lines: &[String]) -> Vec<(usize, usize, usize)> {
    let mut blocks = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let code = strip_comment_and_strings(line);
        let trimmed = code.trim_start();
        if !trimmed.starts_with("if ") || !trimmed.contains("TYPE_CHECKING") {
            continue;
        }
        let indent = indent_width(line);
        let end = block_end(lines, idx + 1, indent);
        blocks.push((idx + 1, end, indent));
    }
    blocks
}

fn is_argumentless_stub_class(class_idx: usize, nodes: &[Node], lines: &[String]) -> bool {
    is_argumentless_class(&nodes[class_idx], lines)
        && direct_methods(class_idx, nodes).is_empty()
        && !class_has_attributes(class_idx, nodes, lines)
        && is_stub_class(class_idx, nodes, lines)
}

fn is_argumentless_class(class: &Node, lines: &[String]) -> bool {
    let Some(line) = lines.get(class.start - 1) else {
        return false;
    };
    let code = strip_comment_and_strings(line);
    let trimmed = code.trim_start();
    let Some(rest) = trimmed.strip_prefix("class ") else {
        return false;
    };
    let after_name = rest
        .trim_start_matches(|ch: char| ch.is_ascii_alphanumeric() || ch == '_')
        .trim_start();
    after_name.starts_with(':')
}

fn has_intervening_top_level_statement(lines: &[String], start: usize, end: usize) -> bool {
    if end < start {
        return false;
    }
    for line_no in start..=end {
        let Some(line) = lines.get(line_no - 1) else {
            continue;
        };
        if indent_width(line) != 0 {
            continue;
        }
        let code = strip_comment_and_strings(line);
        let trimmed = code.trim();
        if trimmed.is_empty() || trimmed.starts_with('@') {
            continue;
        }
        return true;
    }
    false
}

fn is_public_standalone(node: &Node) -> bool {
    matches!(node.kind, NodeKind::Class | NodeKind::Function) && !is_private_name(&node.name)
}

fn is_private_name(name: &str) -> bool {
    name.starts_with('_') && !name.starts_with("__")
}

fn is_private_helper_for_next(
    left_idx: usize,
    right_idx: usize,
    nodes: &[Node],
    source: &str,
) -> bool {
    let left = &nodes[left_idx];
    let right = &nodes[right_idx];
    if !matches!(left.kind, NodeKind::Class | NodeKind::Function) || !is_private_name(&left.name) {
        return false;
    }
    if !matches!(right.kind, NodeKind::Class | NodeKind::Function) {
        return false;
    }
    let right_text = source_lines_range(source, right.start, right.body_end);
    if !right_text.contains(&left.name) {
        return false;
    }
    let rest = source_lines_range(source, right.body_end + 1, usize::MAX);
    !rest.contains(&left.name)
}

fn source_lines_range(source: &str, start: usize, end: usize) -> String {
    source
        .split('\n')
        .enumerate()
        .filter(|(idx, _)| {
            let line_no = idx + 1;
            line_no >= start && line_no <= end
        })
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn run_class_spacing_rules(
    display_path: &str,
    lines: &[String],
    nodes: &[Node],
    config: &EffectiveConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (class_idx, class) in nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.kind == NodeKind::Class)
    {
        let methods = direct_methods(class_idx, nodes);
        let stub = is_stub_class(class_idx, nodes, lines);
        if methods.len() >= 2 {
            for pair in methods.windows(2) {
                let left = &nodes[pair[0]];
                let right = &nodes[pair[1]];
                let actual = blank_count_between(left.end_nonblank, right.group_start);
                if stub {
                    if config.is_enabled("SK311") {
                        push_blank_diagnostic(
                            diagnostics,
                            display_path,
                            lines,
                            left.end_nonblank,
                            right.group_start,
                            actual,
                            blank_rule(
                                "SK311",
                                "Stub class methods must be separated by exactly 1 blank line",
                                "Set 1 blank line between stub class methods",
                                1,
                            ),
                        );
                    }
                } else if class.indent > 0 {
                    if config.is_enabled("SK301") {
                        push_blank_diagnostic(
                            diagnostics,
                            display_path,
                            lines,
                            left.end_nonblank,
                            right.group_start,
                            actual,
                            blank_rule(
                                "SK301",
                                "Nested classes must have exactly 1 blank line between methods",
                                "Set 1 blank line between nested class methods",
                                1,
                            ),
                        );
                    }
                } else if config.is_enabled("SK303") {
                    push_blank_diagnostic(
                        diagnostics,
                        display_path,
                        lines,
                        left.end_nonblank,
                        right.group_start,
                        actual,
                        blank_rule(
                            "SK303",
                            "Regular class methods must be separated by exactly 2 blank lines",
                            "Set 2 blank lines between class methods",
                            2,
                        ),
                    );
                }
            }
        }

        if !stub && class.indent > 0 && config.is_enabled("SK301") {
            run_nested_class_extra_blank_rule(display_path, lines, nodes, class_idx, diagnostics);
        }
    }
}

fn direct_methods(class_idx: usize, nodes: &[Node]) -> Vec<usize> {
    nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.kind == NodeKind::Function && node.parent == Some(class_idx))
        .map(|(idx, _)| idx)
        .collect()
}

fn run_nested_class_extra_blank_rule(
    display_path: &str,
    lines: &[String],
    nodes: &[Node],
    class_idx: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let class = &nodes[class_idx];
    let method_ranges: Vec<(usize, usize)> = direct_methods(class_idx, nodes)
        .into_iter()
        .map(|idx| (nodes[idx].end_nonblank, nodes[idx].group_start))
        .collect();
    let mut prev = first_nonblank_in_range(lines, class.start + 1, class.body_end);
    while let Some(prev_line) = prev {
        let next = first_nonblank_in_range(lines, prev_line + 1, class.body_end);
        let Some(next_line) = next else { break };
        let blanks = blank_count_between(prev_line, next_line);
        if blanks > 0 {
            let is_between_methods = method_ranges
                .iter()
                .any(|(left_end, right_start)| *left_end == prev_line && *right_start == next_line);
            let after_initial_docstring =
                is_gap_after_initial_docstring(lines, class, prev_line, next_line);
            if !is_between_methods && !after_initial_docstring {
                push_blank_diagnostic(
                    diagnostics,
                    display_path,
                    lines,
                    prev_line,
                    next_line,
                    blanks,
                    blank_rule(
                        "SK301",
                        "Nested classes must not contain blank lines outside method separators",
                        "Remove the extra blank line inside the nested class",
                        0,
                    ),
                );
            }
        }
        prev = next;
    }
}

fn is_stub_class(class_idx: usize, nodes: &[Node], lines: &[String]) -> bool {
    let methods = direct_methods(class_idx, nodes);
    let has_attrs = class_has_attributes(class_idx, nodes, lines);
    if methods.is_empty() && !has_attrs {
        return true;
    }
    !methods.is_empty()
        && !has_attrs
        && methods
            .iter()
            .all(|idx| is_stub_function(*idx, nodes, lines))
}

fn class_has_attributes(class_idx: usize, nodes: &[Node], lines: &[String]) -> bool {
    let class = &nodes[class_idx];
    let child_indent = class.indent + 4;
    for line_no in class.start + 1..=class.body_end {
        let line = &lines[line_no - 1];
        if line.trim().is_empty() || indent_width(line) != child_indent {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('@')
            || trimmed.starts_with("def ")
            || trimmed.starts_with("async def ")
            || trimmed.starts_with("class ")
            || trimmed.starts_with("pass")
            || trimmed.starts_with("...")
            || trimmed.starts_with("\"\"\"")
            || trimmed.starts_with("'''")
        {
            continue;
        }
        if trimmed.contains(':') || trimmed.contains('=') {
            return true;
        }
    }
    false
}

fn is_stub_function(function_idx: usize, nodes: &[Node], lines: &[String]) -> bool {
    let function = &nodes[function_idx];
    let body_indent = function.indent + 4;
    let mut in_string = false;
    let mut saw_statement = false;
    for line_no in function.start + 1..=function.body_end {
        let line = &lines[line_no - 1];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.contains("\"\"\"") || trimmed.contains("'''") {
            let count = trimmed.matches("\"\"\"").count() + trimmed.matches("'''").count();
            if count % 2 == 1 {
                in_string = !in_string;
            }
            continue;
        }
        if in_string {
            continue;
        }
        if indent_width(line) < body_indent {
            continue;
        }
        saw_statement = true;
        if trimmed != "pass"
            && trimmed != "..."
            && !trimmed.starts_with("raise NotImplementedError")
        {
            return false;
        }
    }
    saw_statement
}

fn run_stub_ellipsis_docstring_spacing(
    display_path: &str,
    lines: &[String],
    nodes: &[Node],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for node in nodes.iter().filter(|node| node.kind == NodeKind::Function) {
        let body_indent = node.indent + 4;
        let mut ellipsis_line = None;
        for line_no in node.start + 1..=node.body_end {
            let line = &lines[line_no - 1];
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if indent_width(line) != body_indent {
                continue;
            }
            if trimmed == "..." {
                ellipsis_line = Some(line_no);
                continue;
            }
            if is_docstring_start(trimmed) {
                if let Some(left_line) = ellipsis_line {
                    let actual = blank_count_between(left_line, line_no);
                    push_blank_diagnostic(
                        diagnostics,
                        display_path,
                        lines,
                        left_line,
                        line_no,
                        actual,
                        blank_rule(
                            "SK313",
                            "Stub function and method ellipsis must not be separated from the following docstring by a blank line",
                            "Remove the blank line between the ellipsis and the docstring",
                            0,
                        ),
                    );
                }
                break;
            }
            break;
        }
    }
}

fn run_stub_docstring_ellipsis_spacing(
    display_path: &str,
    lines: &[String],
    nodes: &[Node],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for node in nodes.iter().filter(|node| node.kind == NodeKind::Function) {
        let body_indent = node.indent + 4;
        let mut line_no = node.start + 1;
        while line_no <= node.body_end {
            let line = &lines[line_no - 1];
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                line_no += 1;
                continue;
            }
            if indent_width(line) != body_indent || !is_docstring_start(trimmed) {
                break;
            }

            let Some(doc_end) = docstring_end_line(lines, line_no, node.body_end) else {
                break;
            };
            let Some(ellipsis_line) =
                next_meaningful_body_line(lines, doc_end + 1, node.body_end, body_indent)
            else {
                break;
            };
            if lines[ellipsis_line - 1].trim() == "..." {
                let actual = blank_count_between(doc_end, ellipsis_line);
                push_blank_diagnostic(
                    diagnostics,
                    display_path,
                    lines,
                    doc_end,
                    ellipsis_line,
                    actual,
                    blank_rule(
                        "SK315",
                        "Stub function and method docstrings must not be separated from the following ellipsis by a blank line",
                        "Remove the blank line between the docstring and the ellipsis",
                        0,
                    ),
                );
            }
            break;
        }
    }
}

fn docstring_end_line(lines: &[String], start_line: usize, max_line: usize) -> Option<usize> {
    let trimmed = lines.get(start_line - 1)?.trim();
    let quote = if trimmed.starts_with("\"\"\"") {
        "\"\"\""
    } else if trimmed.starts_with("'''") {
        "'''"
    } else {
        return None;
    };
    let after_open = &trimmed[quote.len()..];
    if after_open.contains(quote) {
        return Some(start_line);
    }
    (start_line + 1..=max_line).find(|line_no| lines[*line_no - 1].contains(quote))
}

fn next_meaningful_body_line(
    lines: &[String],
    start_line: usize,
    end_line: usize,
    body_indent: usize,
) -> Option<usize> {
    (start_line..=end_line).find(|line_no| {
        let line = &lines[*line_no - 1];
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with('#') && indent_width(line) == body_indent
    })
}
fn is_docstring_start(trimmed: &str) -> bool {
    trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''")
}

fn run_body_blank_rules(
    display_path: &str,
    lines: &[String],
    nodes: &[Node],
    config: &EffectiveConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (idx, node) in nodes.iter().enumerate() {
        if node.kind == NodeKind::Main {
            if config.is_enabled("SK308") {
                for (start, end, len) in blank_runs_in_body(lines, node) {
                    if len > 1 {
                        push_blank_run_diagnostic(
                    diagnostics,
                    display_path,
                    lines,
                    start,
                    end,
                    blank_rule("SK308", "The __main__ block must not contain more than 1 consecutive blank line", "Reduce blank lines inside the __main__ block to 1", 1),
                );
                    }
                }
            }
            continue;
        }

        if node.kind != NodeKind::Function {
            continue;
        }
        let parent_kind = node.parent.map(|parent| nodes[parent].kind);
        let nested_function = parent_kind == Some(NodeKind::Function);
        if nested_function && config.is_enabled("SK302") {
            for (start, end, _len) in blank_runs_in_body(lines, node) {
                if is_blank_run_after_initial_docstring(lines, node, start, end) {
                    continue;
                }
                push_blank_run_diagnostic(
                    diagnostics,
                    display_path,
                    lines,
                    start,
                    end,
                    blank_rule(
                        "SK302",
                        "Nested function bodies must not contain blank lines",
                        "Remove the blank line from the nested function body",
                        0,
                    ),
                );
            }
        } else if config.is_enabled("SK305") {
            for (start, end, len) in blank_runs_in_body(lines, node) {
                if len > 1 {
                    push_blank_run_diagnostic(
                    diagnostics,
                    display_path,
                    lines,
                    start,
                    end,
                    blank_rule("SK305", "Function and method bodies must not contain more than 1 consecutive blank line", "Reduce consecutive blank lines in the body to 1", 1),
                );
                }
            }
        }

        let _ = idx;
    }
}

fn is_gap_after_initial_docstring(
    lines: &[String],
    node: &Node,
    left_line: usize,
    right_line: usize,
) -> bool {
    let Some(close_line) = initial_docstring_close_line(lines, node) else {
        return false;
    };
    left_line == close_line && right_line > close_line + 1
}

fn is_blank_run_after_initial_docstring(
    lines: &[String],
    node: &Node,
    start: usize,
    end: usize,
) -> bool {
    let Some(close_line) = initial_docstring_close_line(lines, node) else {
        return false;
    };
    start == close_line + 1 && end >= start
}

fn initial_docstring_close_line(lines: &[String], node: &Node) -> Option<usize> {
    let first = first_nonblank_in_range(lines, node.start + 1, node.body_end)?;
    let trimmed = lines[first - 1].trim_start();
    let quote = if trimmed.starts_with("\"\"\"") {
        "\"\"\""
    } else if trimmed.starts_with("\'\'\'") {
        "\'\'\'"
    } else {
        return None;
    };

    if trimmed[quote.len()..].contains(quote) {
        return Some(first);
    }

    (first + 1..=node.body_end).find(|line_no| lines[*line_no - 1].contains(quote))
}

fn blank_runs_in_body(lines: &[String], node: &Node) -> Vec<(usize, usize, usize)> {
    let Some(first) = first_nonblank_in_range(lines, node.start + 1, node.body_end) else {
        return Vec::new();
    };
    let Some(last) = last_nonblank_in_range(lines, first, node.body_end) else {
        return Vec::new();
    };
    let mut runs = Vec::new();
    let mut current_start: Option<usize> = None;
    for line_no in first..=last {
        let is_blank = lines[line_no - 1].trim().is_empty();
        if is_blank && current_start.is_none() {
            current_start = Some(line_no);
        } else if !is_blank {
            if let Some(start) = current_start.take() {
                let end = line_no - 1;
                runs.push((start, end, end - start + 1));
            }
        }
    }
    if let Some(start) = current_start {
        runs.push((start, last, last - start + 1));
    }
    runs
}

fn blank_count_between(left_line: usize, right_line: usize) -> usize {
    right_line.saturating_sub(left_line + 1)
}

#[derive(Debug, Clone, Copy)]
struct BlankLineRule {
    code: &'static str,
    message: &'static str,
    fix_message: &'static str,
    expected: usize,
}

fn blank_rule(
    code: &'static str,
    message: &'static str,
    fix_message: &'static str,
    expected: usize,
) -> BlankLineRule {
    BlankLineRule {
        code,
        message,
        fix_message,
        expected,
    }
}

fn push_blank_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    display_path: &str,
    lines: &[String],
    left_line: usize,
    right_line: usize,
    actual: usize,
    rule: BlankLineRule,
) {
    if actual == rule.expected || left_line == 0 || right_line == 0 || right_line <= left_line {
        return;
    }
    let start_column = lines
        .get(left_line - 1)
        .map(|line| line.chars().count() + 1)
        .unwrap_or(1);
    diagnostics.push(
        Diagnostic::new(
            rule.code,
            rule.message,
            display_path,
            Span::new(left_line, start_column, right_line, 1),
            "warning",
        )
        .with_fix(Fix {
            message: rule.fix_message.to_string(),
            replacement: "\n".repeat(rule.expected + 1),
            start_line: left_line,
            start_column,
            end_line: right_line,
            end_column: 1,
        }),
    );
}

fn push_blank_run_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    display_path: &str,
    lines: &[String],
    start_blank_line: usize,
    end_blank_line: usize,
    rule: BlankLineRule,
) {
    if start_blank_line == 0 || end_blank_line < start_blank_line {
        return;
    }
    let left_line = start_blank_line - 1;
    let right_line = end_blank_line + 1;
    if left_line == 0 || right_line > lines.len() {
        return;
    }
    let start_column = lines[left_line - 1].chars().count() + 1;
    diagnostics.push(
        Diagnostic::new(
            rule.code,
            rule.message,
            display_path,
            Span::new(start_blank_line, 1, end_blank_line, 1),
            "warning",
        )
        .with_fix(Fix {
            message: rule.fix_message.to_string(),
            replacement: "\n".repeat(rule.expected + 1),
            start_line: left_line,
            start_column,
            end_line: right_line,
            end_column: 1,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VscodeConfig;
    use crate::formatter::format_source;
    use std::path::PathBuf;

    fn codes(source: &str) -> Vec<String> {
        let cfg = EffectiveConfig::resolve(
            &VscodeConfig::default(),
            &Default::default(),
            &Default::default(),
        );
        run_blank_line_rules(Path::new("example.py"), source, &cfg)
            .into_iter()
            .map(|diag| diag.code)
            .collect()
    }

    #[test]
    fn catches_nested_class_spacing() {
        let source = "def outer():\n    class Inner:\n        x = 1\n\n        def a(self):\n            pass\n\n\n        def b(self):\n            pass";
        let found = codes(source);
        assert!(found.contains(&"SK301".to_string()));
    }

    #[test]
    fn catches_nested_function_blank_lines() {
        let source = "def outer():\n    def inner():\n        x = 1\n\n        return x";
        assert!(codes(source).contains(&"SK302".to_string()));
    }

    #[test]
    fn ignores_blank_after_initial_docstring_in_nested_class() {
        let source =
            "def outer():\n    class Inner:\n        \"\"\"Описание\"\"\"\n\n        value = 1\n";
        assert!(!codes(source).contains(&"SK301".to_string()));
    }

    #[test]
    fn ignores_blank_after_initial_docstring_in_nested_function() {
        let source =
            "def outer():\n    def inner():\n        \"\"\"Описание\"\"\"\n\n        return 1\n";
        assert!(!codes(source).contains(&"SK302".to_string()));
    }
    #[test]
    fn catches_regular_class_method_spacing() {
        let source =
            "class Box:\n    def a(self):\n        return 1\n\n    def b(self):\n        return 2";
        assert!(codes(source).contains(&"SK303".to_string()));
    }

    #[test]
    fn multiline_method_signature_does_not_break_sk303_ranges() {
        let source = "class Box:
    @staticmethod
    def _parse_configs(
        config_path: Path,
        modules: tuple[type[object], ...]
    ) -> list[dict]:
        class Common:
            value: int

        return []


    def next_method(self):
        return None";
        assert!(!codes(source).contains(&"SK303".to_string()));
    }

    #[test]
    fn catches_public_top_level_spacing() {
        let source = "def a():\n    pass\n\ndef b():\n    pass";
        assert!(codes(source).contains(&"SK306".to_string()));
    }

    #[test]
    fn sk306_does_not_cross_type_checking_block() {
        let source = r#"from typing import TYPE_CHECKING
from abc import ABCMeta

class ABCInitializableMeta:
    """
    Общий метакласс
    """

if TYPE_CHECKING:
    from somewhere import DataDictMeta

    # Комбинированный метакласс для проверки типов
    class _InitializableHandlerMeta(DataDictMeta, ABCInitializableMeta):
        """
        Общий метакласс
        """

    class _ABCHandlerMeta(DataDictMeta, ABCMeta):
        """
        Общий метакласс
        """

    class _ABCInitializableHandlerMeta(ABCInitializableMeta, _InitializableHandlerMeta, _ABCHandlerMeta):
        """
        Общий метакласс
        """

    _ABCMeta = _ABCHandlerMeta
else:
    _ABCMeta = ABCMeta
    _ABCInitializableHandlerMeta = ABCInitializableMeta

class BaseInitializable:
    """
    Базовый класс
    """"#;
        assert!(!codes(source).contains(&"SK306".to_string()));
    }

    #[test]
    fn catches_too_many_body_blank_lines() {
        let source = "def f():\n    x = 1\n\n\n    return x";
        assert!(codes(source).contains(&"SK305".to_string()));
    }

    #[test]
    fn catches_main_spacing_and_body_blank_lines() {
        let source = "def f():\n    pass\n\nif __name__ == \"__main__\":\n    f()\n\n\n    f()";
        let found = codes(source);
        assert!(found.contains(&"SK307".to_string()));
        assert!(found.contains(&"SK308".to_string()));
    }

    #[test]
    fn catches_private_helper_spacing() {
        let source = "def _helper():\n    return 1\n\ndef public():\n    return _helper()";
        assert!(codes(source).contains(&"SK310".to_string()));
    }

    #[test]
    fn catches_top_level_stub_class_spacing() {
        let source = "class A:\n    pass\n\nclass B:\n    pass";
        assert!(codes(source).contains(&"SK312".to_string()));
    }

    #[test]
    fn formatter_fixes_top_level_spacing() {
        let source = "def a():\n    pass\n\ndef b():\n    pass";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );
        assert!(report.source.contains("pass\n\n\n\ndef b"));
    }

    #[test]
    fn catches_final_newline() {
        assert!(codes("x = 1\n").contains(&"SK309".to_string()));
    }

    #[test]
    fn catches_stub_class_spacing() {
        let source = "class A:\n    def a(self):\n        ...\n\n\n    def b(self):\n        ...";
        assert!(codes(source).contains(&"SK311".to_string()));
    }

    #[test]
    fn catches_blank_between_stub_ellipsis_and_docstring() {
        let source = r#"def f():
    ...

    """Описание""""#;
        assert!(codes(source).contains(&"SK313".to_string()));

        let accepted = r#"def f():
    ...
    """Описание""""#;
        assert!(!codes(accepted).contains(&"SK313".to_string()));
    }

    #[test]
    fn catches_blank_between_stub_docstring_and_ellipsis() {
        let source = r#"def f():
    """Описание"""

    ..."#;
        assert!(codes(source).contains(&"SK315".to_string()));
        assert!(!codes(source).contains(&"SK613".to_string()));

        let accepted = r#"def f():
    """Описание"""
    ..."#;
        assert!(!codes(accepted).contains(&"SK315".to_string()));
        assert!(!codes(accepted).contains(&"SK613".to_string()));
    }

    #[test]
    fn catches_blank_between_multiline_stub_docstring_and_ellipsis() {
        let source = r#"def f():
    """
    Описание
    """

    ..."#;
        assert!(codes(source).contains(&"SK315".to_string()));
        assert!(!codes(source).contains(&"SK613".to_string()));
    }

    #[test]
    fn catches_type_checking_argumentless_stub_class_spacing() {
        let source = "from typing import TYPE_CHECKING

if TYPE_CHECKING:
    class A:
        pass


    class B:
        ...";
        assert!(codes(source).contains(&"SK314".to_string()));

        let accepted = "from typing import TYPE_CHECKING

if TYPE_CHECKING:
    class A:
        pass

    class B:
        ...";
        assert!(!codes(accepted).contains(&"SK314".to_string()));
    }

    #[test]
    fn ignores_type_checking_classes_with_bases_or_methods() {
        let source = "from typing import TYPE_CHECKING

if TYPE_CHECKING:
    class A(Base):
        pass


    class B:
        def method(self):
            ...";
        assert!(!codes(source).contains(&"SK314".to_string()));
    }

    #[test]
    fn ignores_defs_inside_multiline_strings_when_checking_spacing() {
        let found = codes(
            "from typing import Final\n\n_TEMPLATE: Final[str] = \"\"\"\ndef generated():\n    pass\n\"\"\"\n\"\"\"Описание шаблона\"\"\"\n\n\n\n@dataclass_transform()\nclass Public:\n    pass\n",
        );
        assert!(!found.contains(&"SK306".to_string()));
    }
}
