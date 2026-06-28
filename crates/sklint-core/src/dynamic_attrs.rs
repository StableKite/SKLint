use crate::config::EffectiveConfig;
use crate::diagnostic::{Diagnostic, Span};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone)]
struct ClassInfo {
    name: String,
    bases: Vec<String>,
    start_line: usize,
    end_line: usize,
    declared_attrs: HashSet<String>,
    methods: Vec<MethodInfo>,
}

#[derive(Debug, Clone)]
struct MethodInfo {
    name: String,
    start_line: usize,
    end_line: usize,
}

#[derive(Debug, Clone)]
struct ConstructedVariable {
    name: String,
    class_idx: usize,
    start_line: usize,
    end_line: usize,
}

pub fn run_dynamic_attribute_rules(
    path: &Path,
    source: &str,
    config: &EffectiveConfig,
) -> Vec<Diagnostic> {
    let sk701_enabled = config.is_enabled("SK701");
    let sk702_enabled = config.is_enabled("SK702");
    if !sk701_enabled && !sk702_enabled {
        return Vec::new();
    }

    let display_path = path.display().to_string();
    let lines: Vec<&str> = source.lines().collect();
    let classes = collect_classes(&lines);
    let mut diagnostics = Vec::new();

    if sk701_enabled {
        run_self_dynamic_attribute_rule(&display_path, &lines, &classes, &mut diagnostics);
    }
    if sk702_enabled {
        run_known_dynamic_object_attribute_rule(&display_path, &lines, &classes, &mut diagnostics);
    }

    diagnostics
}

fn run_self_dynamic_attribute_rule(
    display_path: &str,
    lines: &[&str],
    classes: &[ClassInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (class_idx, class) in classes.iter().enumerate() {
        let mut emitted_attrs = HashSet::new();
        for method in &class.methods {
            if is_initializer_like_method(&method.name) {
                continue;
            }
            for line_no in method.start_line..=method.end_line {
                let Some(line) = lines.get(line_no - 1) else {
                    continue;
                };
                let code_part = strip_inline_comment(line);
                for (attr, start_col, end_col) in self_attribute_assignment_occurrences(code_part) {
                    if class_declares_attr(classes, class_idx, &attr) {
                        continue;
                    }
                    if !emitted_attrs.insert(attr.clone()) {
                        continue;
                    }
                    diagnostics.push(Diagnostic::new(
                        "SK701",
                        format!(
                            "Instance attribute `{attr}` is introduced outside `__init__` in `{}` and is not declared on the class",
                            class.name
                        ),
                        display_path.to_string(),
                        Span::new(line_no, start_col, line_no, end_col),
                        "warning",
                    ));
                }
            }
        }
    }
}

fn run_known_dynamic_object_attribute_rule(
    display_path: &str,
    lines: &[&str],
    classes: &[ClassInfo],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let variables = collect_constructed_variables(lines, classes);
    if variables.is_empty() {
        return;
    }

    let mut emitted = HashSet::new();
    for variable in variables {
        let class = &classes[variable.class_idx];
        if !is_dynamic_attribute_container(classes, variable.class_idx) {
            continue;
        }
        for line_no in variable.start_line..=variable.end_line {
            let Some(line) = lines.get(line_no - 1) else {
                continue;
            };
            let code_part = strip_inline_comment(line);
            for (attr, start_col, end_col) in
                object_attribute_occurrences(code_part, &variable.name)
            {
                if class_declares_attr(classes, variable.class_idx, &attr) {
                    continue;
                }
                let Some(end_index) = byte_index_for_column(code_part, end_col) else {
                    continue;
                };
                if !is_assignment_tail(code_part[end_index..].trim_start()) {
                    continue;
                }
                let key = (line_no, start_col, variable.name.clone(), attr.clone());
                if !emitted.insert(key) {
                    continue;
                }
                diagnostics.push(Diagnostic::new(
                    "SK702",
                    format!(
                        "Attribute `{attr}` is assigned to `{}` but is not declared on `{}` or its known bases",
                        variable.name, class.name
                    ),
                    display_path.to_string(),
                    Span::new(line_no, start_col, line_no, end_col),
                    "warning",
                ));
            }
        }
    }
}

fn collect_constructed_variables(
    lines: &[&str],
    classes: &[ClassInfo],
) -> Vec<ConstructedVariable> {
    let mut variables = Vec::new();
    for line_no in 1..=lines.len() {
        let Some(line) = lines.get(line_no - 1) else {
            continue;
        };
        let code = strip_inline_comment(line).trim_start();
        let Some((variable, rhs)) = parse_simple_assignment(code) else {
            continue;
        };
        let Some(class_name) = parse_constructor_name(rhs) else {
            continue;
        };
        let Some(class_idx) = nearest_class_index(classes, class_name, line_no) else {
            continue;
        };
        let end_line = containing_scope_end(lines, line_no).unwrap_or(lines.len());
        variables.push(ConstructedVariable {
            name: variable.to_string(),
            class_idx,
            start_line: line_no,
            end_line,
        });
    }
    variables
}

fn parse_simple_assignment(code: &str) -> Option<(&str, &str)> {
    let (left, right) = code.split_once('=')?;
    let left = left.trim();
    if !is_identifier(left)
        || left.ends_with(['!', '<', '>', ':', '+', '-', '*', '/', '%', '|', '&', '^'])
    {
        return None;
    }
    if right.starts_with('=') || right.starts_with('>') {
        return None;
    }
    Some((left, right.trim_start()))
}

fn parse_constructor_name(rhs: &str) -> Option<&str> {
    let mut end = 0usize;
    for (idx, ch) in rhs.char_indices() {
        if idx == 0 && !(ch == '_' || ch.is_alphabetic()) {
            return None;
        }
        if !(ch == '_' || ch == '.' || ch.is_alphanumeric()) {
            break;
        }
        end = idx + ch.len_utf8();
    }
    if end == 0 {
        return None;
    }
    let name = &rhs[..end];
    if !rhs[end..].trim_start().starts_with('(') {
        return None;
    }
    Some(simple_name(name))
}

fn containing_scope_end(lines: &[&str], line_no: usize) -> Option<usize> {
    let line_indent = indent_width(lines.get(line_no - 1)?);
    let mut best: Option<(usize, usize)> = None;
    for candidate in (1..line_no).rev() {
        let line = lines[candidate - 1];
        let trimmed = line.trim_start();
        if !(trimmed.starts_with("def ")
            || trimmed.starts_with("async def ")
            || trimmed.starts_with("class "))
        {
            continue;
        }
        let indent = indent_width(line);
        if indent >= line_indent {
            continue;
        }
        let end = block_end(lines, candidate, indent);
        if end >= line_no {
            best = Some((indent, end));
            break;
        }
    }
    best.map(|(_, end)| end)
}

fn nearest_class_index(classes: &[ClassInfo], name: &str, before_line: usize) -> Option<usize> {
    classes
        .iter()
        .enumerate()
        .filter(|(_, class)| class.name == name && class.end_line < before_line)
        .max_by_key(|(_, class)| class.start_line)
        .map(|(idx, _)| idx)
}

fn is_dynamic_attribute_container(classes: &[ClassInfo], class_idx: usize) -> bool {
    let class = &classes[class_idx];
    class.methods.iter().any(|method| {
        method.name == "__setattr__"
            || method.name == "__getattr__"
            || method.name == "__getattribute__"
    }) || class.bases.iter().any(|base| {
        let base = simple_name(base);
        matches!(base, "DataDict" | "BaseConcConfig")
            || resolve_base_class(classes, class_idx, base)
                .is_some_and(|base_idx| is_dynamic_attribute_container(classes, base_idx))
    })
}

fn class_declares_attr(classes: &[ClassInfo], class_idx: usize, attr: &str) -> bool {
    let class = &classes[class_idx];
    class.declared_attrs.contains(attr)
        || class.bases.iter().any(|base| {
            resolve_base_class(classes, class_idx, simple_name(base))
                .is_some_and(|base_idx| class_declares_attr(classes, base_idx, attr))
        })
}

fn resolve_base_class(classes: &[ClassInfo], class_idx: usize, base_name: &str) -> Option<usize> {
    let class_start = classes[class_idx].start_line;
    classes
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.name == base_name && candidate.start_line < class_start)
        .max_by_key(|(_, candidate)| candidate.start_line)
        .map(|(idx, _)| idx)
}

fn simple_name(text: &str) -> &str {
    text.rsplit('.').next().unwrap_or(text)
}

fn collect_classes(lines: &[&str]) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    for idx in 0..lines.len() {
        let line = lines[idx];
        let trimmed = line.trim_start();
        let Some((name, bases)) = parse_class_signature(trimmed) else {
            continue;
        };

        let class_indent = indent_width(line);
        let class_start = idx + 1;
        let class_end = block_end(lines, class_start, class_indent);
        let methods = collect_methods(lines, class_start + 1, class_end, class_indent);
        let declared_attrs =
            collect_declared_attrs(lines, class_start + 1, class_end, class_indent, &methods);

        classes.push(ClassInfo {
            name,
            bases,
            start_line: class_start,
            end_line: class_end,
            declared_attrs,
            methods,
        });
    }
    classes
}

fn collect_methods(
    lines: &[&str],
    start_line: usize,
    end_line: usize,
    class_indent: usize,
) -> Vec<MethodInfo> {
    let mut methods = Vec::new();
    let mut line_no = start_line;
    while line_no <= end_line {
        let Some(line) = lines.get(line_no - 1) else {
            break;
        };
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@') {
            line_no += 1;
            continue;
        }
        let indent = indent_width(line);
        if indent == class_indent + 4 {
            if let Some(name) = parse_def_name(trimmed) {
                let method_end = block_end(lines, line_no, indent);
                methods.push(MethodInfo {
                    name,
                    start_line: line_no + 1,
                    end_line: method_end,
                });
                line_no = method_end + 1;
                continue;
            }
        }
        line_no += 1;
    }
    methods
}

fn collect_declared_attrs(
    lines: &[&str],
    start_line: usize,
    end_line: usize,
    class_indent: usize,
    methods: &[MethodInfo],
) -> HashSet<String> {
    let mut attrs = HashSet::new();
    attrs.insert("__class__".to_string());
    attrs.insert("__dict__".to_string());

    for line_no in start_line..=end_line {
        let Some(line) = lines.get(line_no - 1) else {
            continue;
        };
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@') {
            continue;
        }
        let indent = indent_width(line);
        if indent == class_indent + 4 {
            let code = strip_inline_comment(trimmed);
            if let Some(attr) = parse_class_attr_name(code) {
                attrs.insert(attr);
            }
            if let Some(method) = parse_def_name(code) {
                attrs.insert(method);
            }
        }
    }

    for method in methods {
        if !is_initializer_like_method(&method.name) {
            continue;
        }
        for line_no in method.start_line..=method.end_line {
            let Some(line) = lines.get(line_no - 1) else {
                continue;
            };
            let code_part = strip_inline_comment(line);
            for attr in self_attribute_assignments(code_part) {
                attrs.insert(attr);
            }
        }
    }

    attrs
}

fn self_attribute_assignments(line: &str) -> Vec<String> {
    self_attribute_assignment_occurrences(line)
        .into_iter()
        .map(|(attr, _start_col, _end_col)| attr)
        .collect()
}

fn self_attribute_assignment_occurrences(line: &str) -> Vec<(String, usize, usize)> {
    object_attribute_occurrences(line, "self")
        .into_iter()
        .filter_map(|(attr, start_col, end_col)| {
            let end_index = byte_index_for_column(line, end_col)?;
            let rest = line[end_index..].trim_start();
            if is_assignment_tail(rest) {
                Some((attr, start_col, end_col))
            } else {
                None
            }
        })
        .collect()
}

fn is_initializer_like_method(name: &str) -> bool {
    matches!(name, "__init__" | "__post_init__" | "__setstate__")
}

fn is_assignment_tail(rest: &str) -> bool {
    (rest.starts_with('=') && !rest.starts_with("==") && !rest.starts_with("=>"))
        || rest.starts_with(':')
        || rest.starts_with("+=")
        || rest.starts_with("-=")
        || rest.starts_with("*=")
        || rest.starts_with("/=")
        || rest.starts_with("//=")
        || rest.starts_with("%=")
        || rest.starts_with("**=")
        || rest.starts_with("|=")
        || rest.starts_with("&=")
        || rest.starts_with("^=")
}

fn parse_class_attr_name(code: &str) -> Option<String> {
    let left = code
        .split_once(':')
        .map(|(left, _)| left)
        .or_else(|| code.split_once('=').map(|(left, _)| left))?
        .trim();
    if is_identifier(left) {
        Some(left.to_string())
    } else {
        None
    }
}

fn block_end(lines: &[&str], header_line: usize, header_indent: usize) -> usize {
    let mut end = header_line;
    for line_no in header_line + 1..=lines.len() {
        let Some(line) = lines.get(line_no - 1) else {
            break;
        };
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            end = line_no;
            continue;
        }
        let indent = indent_width(line);
        if indent <= header_indent {
            break;
        }
        end = line_no;
    }
    end
}

fn object_attribute_occurrences(line: &str, object: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let pattern = format!("{object}.");
    let string_ranges = string_literal_ranges(line);
    let mut offset = 0usize;
    while let Some(pos) = line[offset..].find(&pattern) {
        let start = offset + pos;
        if is_byte_in_ranges(start, &string_ranges) {
            offset = start + pattern.len();
            continue;
        }
        if start > 0 {
            let prev = line[..start].chars().next_back().unwrap_or(' ');
            if prev == '_' || prev.is_alphanumeric() {
                offset = start + pattern.len();
                continue;
            }
        }
        let attr_start = start + pattern.len();
        let attr_text = &line[attr_start..];
        let mut attr_end = attr_start;
        for (rel, ch) in attr_text.char_indices() {
            if rel == 0 && !(ch == '_' || ch.is_alphabetic()) {
                break;
            }
            if rel > 0 && !(ch == '_' || ch.is_alphanumeric()) {
                break;
            }
            attr_end = attr_start + rel + ch.len_utf8();
        }
        if attr_end > attr_start {
            let attr = line[attr_start..attr_end].to_string();
            let start_col = line[..attr_start].chars().count() + 1;
            let end_col = line[..attr_end].chars().count() + 1;
            out.push((attr, start_col, end_col));
        }
        offset = attr_start.max(start + 1);
    }
    out
}

fn string_literal_ranges(line: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut iter = line.char_indices().peekable();
    while let Some((idx, ch)) = iter.next() {
        if ch != '\'' && ch != '"' {
            continue;
        }
        let prefix = line[..idx].chars().next_back();
        if matches!(prefix, Some('r' | 'R' | 'f' | 'F' | 'b' | 'B' | 'u' | 'U')) {
            // Keep the range start at the quote. The optional prefix is not relevant for
            // attribute occurrence matching.
        }
        let quote = ch;
        let triple = line[idx..].starts_with(&quote.to_string().repeat(3));
        let start = idx;
        let mut end = line.len();
        let mut escaped = false;
        if triple {
            iter.next();
            iter.next();
        }
        while let Some((next_idx, next_ch)) = iter.next() {
            if escaped {
                escaped = false;
                continue;
            }
            if next_ch == '\\' {
                escaped = true;
                continue;
            }
            if triple {
                if next_ch == quote && line[next_idx..].starts_with(&quote.to_string().repeat(3)) {
                    iter.next();
                    iter.next();
                    end = next_idx + 3;
                    break;
                }
            } else if next_ch == quote {
                end = next_idx + quote.len_utf8();
                break;
            }
        }
        ranges.push((start, end));
    }
    ranges
}

fn is_byte_in_ranges(byte_idx: usize, ranges: &[(usize, usize)]) -> bool {
    ranges
        .iter()
        .any(|(start, end)| byte_idx >= *start && byte_idx < *end)
}

fn byte_index_for_column(text: &str, column: usize) -> Option<usize> {
    if column == 0 {
        return None;
    }
    let target_chars = column - 1;
    if target_chars == text.chars().count() {
        return Some(text.len());
    }
    text.char_indices().nth(target_chars).map(|(idx, _ch)| idx)
}

fn parse_class_signature(trimmed: &str) -> Option<(String, Vec<String>)> {
    let text = trimmed.strip_prefix("class ")?;
    let name_end = text.find(['(', ':']).unwrap_or(text.len());
    let name = text[..name_end].trim();
    if !is_identifier(name) {
        return None;
    }

    let bases = text
        .get(name_end..)
        .and_then(|tail| tail.strip_prefix('('))
        .and_then(|tail| tail.split_once(')').map(|(inside, _)| inside))
        .map(|inside| {
            inside
                .split(',')
                .map(str::trim)
                .filter(|base| !base.is_empty())
                .map(|base| {
                    base.split_once('[')
                        .map(|(head, _)| head)
                        .unwrap_or(base)
                        .trim()
                })
                .filter(|base| is_qualified_identifier(base))
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    Some((name.to_string(), bases))
}

fn parse_def_name(trimmed: &str) -> Option<String> {
    let text = trimmed
        .strip_prefix("async def ")
        .or_else(|| trimmed.strip_prefix("def "))?;
    let name = text.split_once('(')?.0.trim();
    if is_identifier(name) {
        Some(name.to_string())
    } else {
        None
    }
}

fn strip_inline_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '#' => return &line[..idx],
            '\'' | '"' => quote = Some(ch),
            _ => {}
        }
    }
    line
}

fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum()
}

fn is_qualified_identifier(text: &str) -> bool {
    !text.is_empty() && text.split('.').all(is_identifier)
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic()) && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EffectiveConfig, FileInlineConfig, PyProjectConfig, VscodeConfig};

    fn config() -> EffectiveConfig {
        config_with_select(&["SK701"])
    }

    fn config_with_select(select: &[&str]) -> EffectiveConfig {
        let mut inline = FileInlineConfig::default();
        inline
            .select
            .extend(select.iter().map(|code| (*code).to_string()));
        EffectiveConfig::resolve(
            &VscodeConfig::default(),
            &PyProjectConfig::default(),
            &inline,
        )
    }

    #[test]
    fn catches_self_attribute_introduced_outside_init() {
        let source = "class Box:\n    def create(self):\n        self.dynamic = 1\n\n    def read(self):\n        return self.dynamic\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert!(diagnostics
            .iter()
            .any(|diag| diag.code == "SK701" && diag.line == 3));
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diag| diag.code == "SK701")
                .count(),
            1
        );
    }

    #[test]
    fn ignores_declared_instance_attributes() {
        let source = "class Box:\n    def __init__(self):\n        self.value = 1\n\n    def read(self):\n        return self.value\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_object_attributes_that_pyright_already_reports() {
        let source = "class Box:\n    pass\n\nbox = Box()\nbox.dynamic = 1\nprint(box.dynamic)\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn catches_dynamic_attribute_on_datadict_like_local_instance() {
        let source = "class Common(BaseConcConfig):\n    pass\n\ncommon = Common()\ncommon.camera_config = CameraConfig()\n";
        let diagnostics = run_dynamic_attribute_rules(
            Path::new("example.py"),
            source,
            &config_with_select(&["SK702"]),
        );
        assert!(diagnostics
            .iter()
            .any(|diag| diag.code == "SK702" && diag.line == 5 && diag.column == 8));
    }

    #[test]
    fn catches_dynamic_attribute_when_later_same_named_class_declares_it() {
        let source = "def first():\n    class Common(BaseConcConfig):\n        pass\n\n    common = Common()\n    common.camera_config = CameraConfig()\n\ndef second():\n    class Common(DataDict):\n        camera_config: CameraConfig | None = None\n";
        let diagnostics = run_dynamic_attribute_rules(
            Path::new("example.py"),
            source,
            &config_with_select(&["SK702"]),
        );
        assert!(diagnostics
            .iter()
            .any(|diag| diag.code == "SK702" && diag.line == 6));
    }

    #[test]
    fn ignores_declared_dynamic_object_attributes() {
        let source = "class Common(BaseConcConfig):\n    camera_config: CameraConfig | None = None\n\ncommon = Common()\ncommon.camera_config = CameraConfig()\n";
        let diagnostics = run_dynamic_attribute_rules(
            Path::new("example.py"),
            source,
            &config_with_select(&["SK702"]),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_dynamic_object_attribute_reads_and_string_literals() {
        let source = "class Common(BaseConcConfig):\n    pass\n\nconfig = Common()\nif config.conc_type == 'async':\n    pass\nprint(f'config.conc_type = {config.conc_type}')\n";
        let diagnostics = run_dynamic_attribute_rules(
            Path::new("example.py"),
            source,
            &config_with_select(&["SK702"]),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_plain_object_attributes_that_pyright_reports() {
        let source =
            "class Common:\n    pass\n\ncommon = Common()\ncommon.camera_config = CameraConfig()\n";
        let diagnostics = run_dynamic_attribute_rules(
            Path::new("example.py"),
            source,
            &config_with_select(&["SK702"]),
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_class_body_annotations() {
        let source =
            "class Box:\n    value: int\n\n    def read(self):\n        return self.value\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert!(diagnostics.is_empty());
    }
}
