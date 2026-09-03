use crate::blank_lines::run_blank_line_rules;
use crate::comments::run_comment_rules;
use crate::config::{load_pyproject_for_file, parse_inline_config, EffectiveConfig, VscodeConfig};
use crate::diagnostic::{Diagnostic, Fix, Span};
use crate::docstrings::run_docstring_rules;
use crate::dynamic_attrs::run_dynamic_attribute_rules;
use crate::magic_constants::run_magic_constant_rule;
use crate::pydoclint::run_pydoclint_rules;
use crate::rules::rule_by_code;
use crate::suppression::{unused_selector_replacement, SuppressionState};
use crate::syntax_rules::run_syntax_rules;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AnalysisInput {
    pub path: PathBuf,
    pub source: String,
    pub vscode_config: VscodeConfig,
}

#[derive(Debug, Clone)]
pub struct AnalysisReport {
    pub diagnostics: Vec<Diagnostic>,
    pub config: EffectiveConfig,
}

pub fn analyze(input: AnalysisInput) -> AnalysisReport {
    let inline = parse_inline_config(&input.source);
    let pyproject = load_pyproject_for_file(&input.path);
    let config = EffectiveConfig::resolve(&input.vscode_config, &pyproject, &inline);
    let mut suppressions = SuppressionState::parse(&input.source);
    let mut visible = Vec::new();

    for diagnostic in run_rules(&input.path, &input.source, &config) {
        if diagnostic.code == "SK805" {
            visible.push(diagnostic);
            continue;
        }

        let suppression_line = diagnostic.suppression_line.unwrap_or(diagnostic.line);
        if let Some(responsible) =
            suppressions.responsible_match_for(suppression_line, &diagnostic.code, None)
        {
            suppressions.mark_match(responsible);
        } else {
            visible.push(diagnostic);
        }
    }

    if config.is_enabled("SK900") {
        for suppression in suppressions.suppressions.clone() {
            let selectors = if suppression.codes.is_empty() {
                vec![None]
            } else {
                (0..suppression.codes.len()).map(Some).collect::<Vec<_>>()
            };

            for selector_index in selectors {
                if suppressions.selector_is_used(suppression.id, selector_index) {
                    continue;
                }
                if suppressions
                    .responsible_match_for(suppression.line, "SK900", Some(suppression.id))
                    .is_some()
                {
                    continue;
                }

                let selector_label = selector_index
                    .and_then(|index| suppression.codes.get(index))
                    .map(String::as_str)
                    .unwrap_or("ALL");
                let rule = rule_by_code("SK900").expect("SK900 exists");
                let source_line = input
                    .source
                    .lines()
                    .nth(suppression.line.saturating_sub(1))
                    .unwrap_or_default();
                let mut diagnostic = Diagnostic::new(
                    rule.code,
                    format!(
                        "Unused SKLint suppression selector `{selector_label}` in `{}`",
                        suppression.text
                    ),
                    input.path.display().to_string(),
                    Span::new(
                        suppression.line,
                        1,
                        suppression.line,
                        source_line.chars().count().saturating_add(1).max(1),
                    ),
                    "warning",
                );
                if let Some(replacement) =
                    unused_selector_replacement(source_line, &suppression, selector_index)
                {
                    diagnostic = diagnostic.with_fix(Fix {
                        safe: true,
                        message: format!(
                            "Remove unused SKLint suppression selector `{selector_label}`"
                        ),
                        replacement,
                        start_line: suppression.line,
                        start_column: 1,
                        end_line: suppression.line,
                        end_column: source_line.chars().count() + 1,
                    });
                }
                visible.push(diagnostic);
            }
        }
    }

    sort_diagnostics_preserving_pydoclint_order(&mut visible);

    AnalysisReport {
        diagnostics: visible,
        config,
    }
}

fn sort_diagnostics_preserving_pydoclint_order(diagnostics: &mut [Diagnostic]) {
    let pydoclint_order = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.starts_with("SKD"))
        .cloned()
        .collect::<Vec<_>>();

    diagnostics.sort_by(|a, b| {
        (a.path.as_str(), a.line, a.column, a.code.as_str()).cmp(&(
            b.path.as_str(),
            b.line,
            b.column,
            b.code.as_str(),
        ))
    });

    // Upstream pydoclint exposes Visitor emission order, which is not always
    // numeric-code order (for example DOC107 can precede DOC103 on the same
    // function). Preserve SKLint's normal sorted diagnostic slots while
    // restoring the relative SKD sequence inside those slots.
    let mut pydoclint_order = pydoclint_order.into_iter();
    for diagnostic in diagnostics
        .iter_mut()
        .filter(|diagnostic| diagnostic.code.starts_with("SKD"))
    {
        *diagnostic = pydoclint_order
            .next()
            .expect("pydoclint diagnostic count is unchanged by sorting");
    }
}

fn is_allowed_docstring_markdown_break(lines: &[&str], idx: usize) -> bool {
    let Some((start, end)) = docstring_range_containing(lines, idx) else {
        return false;
    };
    if idx <= start || idx + 1 >= end {
        return false;
    }
    let Some(next_idx) = (idx + 1..end).find(|line_idx| !lines[*line_idx].trim().is_empty()) else {
        return false;
    };
    let current = lines[idx].trim_end_matches([' ', '\t']);
    let next = lines[next_idx];
    let next_trimmed = next.trim_start();
    if next_trimmed.is_empty() || next_trimmed.ends_with(':') {
        return false;
    }
    if next_trimmed.chars().next().is_some_and(is_cyrillic_lower) {
        return false;
    }
    indent_width(next) >= indent_width(current)
}

fn docstring_range_containing(lines: &[&str], idx: usize) -> Option<(usize, usize)> {
    let mut scan = 0usize;
    while scan < lines.len() {
        let trimmed = lines[scan].trim_start();
        let quote = if trimmed.starts_with("\"\"\"") {
            "\"\"\""
        } else if trimmed.starts_with("'''") {
            "'''"
        } else {
            scan += 1;
            continue;
        };

        if trimmed[quote.len()..].contains(quote) {
            scan += 1;
            continue;
        }

        for (end, line) in lines.iter().enumerate().skip(scan + 1) {
            if line.contains(quote) {
                if scan <= idx && idx <= end {
                    return Some((scan, end));
                }
                scan = end + 1;
                break;
            }
        }
        scan += 1;
    }
    None
}

fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum()
}

fn is_cyrillic_lower(ch: char) -> bool {
    (('\u{0400}'..='\u{04FF}').contains(&ch) || ('\u{0500}'..='\u{052F}').contains(&ch))
        && ch.is_lowercase()
}

fn run_file_wide_suppression_rule(path: &Path, source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let display_path = path.display().to_string();
    let lines: Vec<&str> = source.lines().collect();
    let mut idx = 0usize;

    while idx < lines.len() {
        let trimmed = lines[idx].trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            inspect_file_wide_suppression_line(
                &mut diagnostics,
                &display_path,
                idx + 1,
                lines[idx],
            );
            idx += 1;
            continue;
        }
        break;
    }

    if let Some(end_idx) = module_docstring_end(&lines, idx) {
        idx = end_idx + 1;
        while idx < lines.len() {
            let trimmed = lines[idx].trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                inspect_file_wide_suppression_line(
                    &mut diagnostics,
                    &display_path,
                    idx + 1,
                    lines[idx],
                );
                idx += 1;
                continue;
            }
            break;
        }
    }

    diagnostics
}

fn inspect_file_wide_suppression_line(
    diagnostics: &mut Vec<Diagnostic>,
    display_path: &str,
    line_no: usize,
    line: &str,
) {
    let Some(comment_start) = line.find('#') else {
        return;
    };

    if !line[..comment_start].trim().is_empty() {
        return;
    }

    for (segment_start, segment_end, segment) in hash_comment_segments(&line[comment_start..]) {
        let Some(kind) = file_wide_suppression_kind(segment) else {
            continue;
        };
        let absolute_start = comment_start + segment_start;
        let absolute_end = comment_start + segment_end;
        let column = line[..absolute_start].chars().count() + 1;
        let segment_end_column = line[..absolute_end].chars().count() + 1;
        let replacement = remove_hash_comment_segment(line, absolute_start, absolute_end);
        diagnostics.push(
            Diagnostic::new(
                "SK805",
                format!("File-wide `{kind}` suppression is forbidden in strict mode"),
                display_path.to_string(),
                Span::new(line_no, column, line_no, segment_end_column.max(column + 1)),
                "warning",
            )
            .with_fix(Fix {
                safe: true,
                message: format!("Remove forbidden file-wide `{kind}` suppression"),
                replacement,
                start_line: line_no,
                start_column: 1,
                end_line: line_no,
                end_column: line.chars().count() + 1,
            }),
        );
    }
}

fn module_docstring_end(lines: &[&str], start_idx: usize) -> Option<usize> {
    let first = lines.get(start_idx)?.trim_start();
    let quote = triple_quote_prefix(first)?;
    let after_opening = &first[quote.len()..];
    if after_opening.contains(quote) {
        return Some(start_idx);
    }
    (start_idx + 1..lines.len()).find(|idx| lines[*idx].contains(quote))
}

fn triple_quote_prefix(trimmed: &str) -> Option<&'static str> {
    let without_prefix = trimmed
        .strip_prefix('r')
        .or_else(|| trimmed.strip_prefix('R'))
        .or_else(|| trimmed.strip_prefix('u'))
        .or_else(|| trimmed.strip_prefix('U'))
        .unwrap_or(trimmed);
    if without_prefix.starts_with("\"\"\"") {
        Some("\"\"\"")
    } else if without_prefix.starts_with("'''") {
        Some("'''")
    } else {
        None
    }
}

fn hash_comment_segments(comment: &str) -> Vec<(usize, usize, &str)> {
    let starts = comment
        .match_indices('#')
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    starts
        .iter()
        .enumerate()
        .map(|(position, start)| {
            let end = starts.get(position + 1).copied().unwrap_or(comment.len());
            (*start, end, &comment[*start..end])
        })
        .collect()
}

fn remove_hash_comment_segment(line: &str, start: usize, end: usize) -> String {
    let leading_indent = line
        .chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .collect::<String>();
    let before = line[..start].trim_end_matches([' ', '\t']);
    let after = line[end..].trim_start_matches([' ', '\t']);

    match (before.trim().is_empty(), after.is_empty()) {
        (true, true) => String::new(),
        (true, false) => format!("{leading_indent}{after}"),
        (false, true) => before.to_string(),
        (false, false) => format!("{before}  {after}"),
    }
}

fn file_wide_suppression_kind(comment: &str) -> Option<&'static str> {
    let text = comment.trim_start().strip_prefix('#')?.trim_start();
    let text = text.split('#').next().unwrap_or(text).trim();
    let lower = text.to_ascii_lowercase();

    if is_noqa_file_suppression(&lower) {
        return Some("noqa");
    }
    if let Some(body) = lower.strip_prefix("pylint:") {
        let body = body.trim_start();
        if starts_directive_name(body, "disable") || starts_directive_name(body, "skip-file") {
            return Some("pylint");
        }
    }
    if let Some(body) = lower.strip_prefix("pyright:") {
        let body = body.trim_start();
        if body.starts_with("ignore") || contains_disabled_pyright_report(body) {
            return Some("pyright");
        }
    }
    if lower.starts_with("type: ignore") {
        return Some("type-ignore");
    }
    if let Some(body) = lower.strip_prefix("mypy:") {
        let body = body.trim_start();
        if body.contains("ignore-errors") || body.contains("disable-error-code") {
            return Some("mypy");
        }
    }
    if let Some(body) = lower.strip_prefix("pytype:") {
        let body = body.trim_start();
        if starts_directive_name(body, "disable") || starts_directive_name(body, "skip-file") {
            return Some("pytype");
        }
    }
    if lower.starts_with("pyre-ignore-all-errors") {
        return Some("pyre");
    }
    if lower
        .strip_prefix("isort:")
        .is_some_and(|body| body.trim_start().starts_with("skip_file"))
    {
        return Some("isort");
    }
    if let Some(body) = lower.strip_prefix("sklint") {
        let body = body.trim_start();
        if let Some(body) = body.strip_prefix(':') {
            let body = body.trim_start();
            if starts_directive_name(body, "noqa")
                || starts_directive_name(body, "ignore")
                || starts_directive_name(body, "disable")
            {
                return Some("sklint");
            }
        }
    }

    None
}

fn is_noqa_file_suppression(lower: &str) -> bool {
    starts_directive_name(lower, "noqa")
        || starts_directive_name(lower, "flake8: noqa")
        || starts_directive_name(lower, "ruff: noqa")
}

fn starts_directive_name(text: &str, name: &str) -> bool {
    if text == name {
        return true;
    }
    let Some(rest) = text.strip_prefix(name) else {
        return false;
    };
    rest.starts_with(':')
        || rest.starts_with('=')
        || rest.chars().next().is_some_and(char::is_whitespace)
}

fn contains_disabled_pyright_report(body: &str) -> bool {
    body.split(',').any(|part| {
        let part = part.trim();
        part.starts_with("report")
            && (part.ends_with("=false")
                || part.ends_with("=none")
                || part.ends_with("=\"none\"")
                || part.ends_with("='none'"))
    })
}

fn run_rules(path: &Path, source: &str, config: &EffectiveConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let display_path = path.display().to_string();

    if config.strict {
        diagnostics.extend(run_file_wide_suppression_rule(path, source));
    }

    if config.is_enabled("SK001") {
        let source_lines: Vec<&str> = source.lines().collect();
        for (idx, line) in source_lines.iter().enumerate() {
            let line_no = idx + 1;
            let trimmed = line.trim_end_matches([' ', '\t']);
            if trimmed.len() != line.len() {
                let trailing = &line[trimmed.len()..];
                if trailing == "  " && is_allowed_docstring_markdown_break(&source_lines, idx) {
                    continue;
                }
                let start_column = trimmed.chars().count() + 1;
                let end_column = line.chars().count() + 1;
                diagnostics.push(
                    Diagnostic::new(
                        "SK001",
                        "Trailing spaces or tabs are not allowed",
                        display_path.clone(),
                        Span::new(line_no, start_column, line_no, end_column),
                        "information",
                    )
                    .with_fix(Fix {
                        safe: true,
                        message: "Remove trailing whitespace".to_string(),
                        replacement: String::new(),
                        start_line: line_no,
                        start_column,
                        end_line: line_no,
                        end_column,
                    }),
                );
            }
        }
    }

    if config.is_enabled("SK101") {
        for (idx, line) in source.lines().enumerate() {
            if let Some(comment_idx) = line.find('#') {
                let comment = &line[comment_idx..];
                if let Some(todo_idx) = comment.to_ascii_lowercase().find("todo") {
                    let line_no = idx + 1;
                    let column = line[..comment_idx].chars().count() + todo_idx + 1;
                    diagnostics.push(Diagnostic::new(
                        "SK101",
                        "TODO comments are not allowed in strict mode",
                        display_path.clone(),
                        Span::new(line_no, column, line_no, column + 4),
                        "warning",
                    ));
                }
            }
        }
    }

    diagnostics.extend(run_comment_rules(path, source, config));
    diagnostics.extend(run_blank_line_rules(path, source, config));
    diagnostics.extend(run_docstring_rules(path, source, config));
    diagnostics.extend(run_pydoclint_rules(path, source, config));
    diagnostics.extend(run_dynamic_attribute_rules(path, source, config));
    diagnostics.extend(run_magic_constant_rule(path, source, config));
    diagnostics.extend(run_syntax_rules(path, source, config));

    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn input(source: &str) -> AnalysisInput {
        AnalysisInput {
            path: PathBuf::from("example.py"),
            source: source.to_string(),
            vscode_config: VscodeConfig::default(),
        }
    }

    #[test]
    fn pydoclint_diagnostics_preserve_upstream_emission_order() {
        let report = analyze(input(
            r#"# sklint: strict
def f(a, b: int):
    """Summary.

    Args:
        a (str): A.
        c (int): C.
    """
    pass
"#,
        ));
        let codes = report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.starts_with("SKD"))
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>();
        let pos_107 = codes
            .iter()
            .position(|code| *code == "SKD107")
            .expect("SKD107");
        let pos_103 = codes
            .iter()
            .position(|code| *code == "SKD103")
            .expect("SKD103");
        assert!(pos_107 < pos_103, "upstream emits DOC107 before DOC103");
    }

    #[test]
    fn finds_trailing_whitespace() {
        let report = analyze(input("x = 1  \n"));
        assert!(report.diagnostics.iter().any(|diag| diag.code == "SK001"));
    }

    #[test]
    fn trailing_whitespace_is_information_level() {
        let report = analyze(input("x = 1  \n"));
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diag| diag.code == "SK001")
            .expect("SK001 exists");
        assert_eq!(diagnostic.level, "information");
    }

    #[test]
    fn noqa_suppresses_and_is_not_unused() {
        let report = analyze(input("x = 1  # noqa: SK001  \n"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK001"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK900"));
    }

    #[test]
    fn t201_noqa_suppresses_sk201_without_unused_suppression() {
        let report = analyze(input("print('debug')  # noqa: T201\n"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK201"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK900"));
    }

    #[test]
    fn sk201_noqa_still_suppresses_print_rule() {
        let report = analyze(input("print('debug')  # noqa: SK201\n"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK201"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK900"));
    }

    #[test]
    fn sk900_reports_only_redundant_selector_and_offers_safe_fix() {
        let report = analyze(input("x=1  # noqa: E501, SK4, SK401\n"));
        let unused = report
            .diagnostics
            .iter()
            .filter(|diag| diag.code == "SK900")
            .collect::<Vec<_>>();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("SK4"));
        let fix = unused[0].fix.as_ref().expect("safe selective fix");
        assert!(fix.safe);
        assert_eq!(fix.replacement, "x=1  # noqa: E501, SK401");
    }

    #[test]
    fn used_catch_all_suppression_is_not_reported_unused() {
        let report = analyze(input("x=1  # sklint: ignore\n"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK401"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK900"));
    }

    #[test]
    fn unused_noqa_is_reported() {
        let report = analyze(input("x = 1  # noqa: SK001\n"));
        assert!(report.diagnostics.iter().any(|diag| diag.code == "SK900"));
    }

    #[test]
    fn docstring_suppression_uses_closing_line() {
        let report = analyze(input(
            "def f():\n    \"\"\"Function loads value.\"\"\"  # noqa: SK604, SK612, SK603, SK613\n    ...\n",
        ));
        assert!(report
            .diagnostics
            .iter()
            .all(|diag| !diag.code.starts_with("SK6")));
    }

    #[test]
    fn doc_prefix_noqa_on_docstring_closing_line_suppresses_skd203() {
        let report = analyze(input(
            r#"# sklint: strict
def f() -> int:
    """Summary.

    Returns:
        str: Value.
    """  # noqa: DOC2
    return 1
"#,
        ));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SKD203"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK900"));
    }

    #[test]
    fn explanatory_prefix_noqa_on_docstring_closing_line_matches_upstream() {
        let report = analyze(input(
            r#"# sklint: strict
def funcDocstringComment(arg1: int, arg2: int) -> None:
    """Demonstrate docstring comment suppression.

    Args:
        arg1 (int): Documented argument.

    """  # explanation noqa: doc101, doc103, F401 trailing words
    pass
"#,
        ));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SKD101"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SKD103"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK900"));
    }

    #[test]
    fn bare_noqa_on_docstring_closing_line_does_not_suppress_skd203() {
        let report = analyze(input(
            r#"# sklint: strict
def f() -> int:
    """Summary.

    Returns:
        str: Value.
    """  # noqa
    return 1
"#,
        ));
        assert!(report.diagnostics.iter().any(|diag| diag.code == "SKD203"));
    }

    #[test]
    fn sk001_allows_intentional_docstring_markdown_break() {
        let report = analyze(input(
            r#"def f():
    """
    Описание

    Returns:
        tuple[int, int]: первое значение  
            Второе значение
    """
    pass
"#,
        ));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK001"));
    }

    #[test]
    fn sk001_reports_two_spaces_before_lowercase_continuation() {
        let report = analyze(input(
            r#"def f():
    """
    Описание

    Returns:
        tuple[int, int]: первое значение  
            второе значение
    """
    pass
"#,
        ));
        assert!(report.diagnostics.iter().any(|diag| diag.code == "SK001"));
    }

    #[test]
    fn noqa_after_existing_comment_suppresses_diagnostic() {
        let report = analyze(input("x=1  # pyright: ignore[reportAny]  # noqa: SK401\n"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK401"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK900"));
    }

    #[test]
    fn sklint_ignore_after_existing_comment_suppresses_diagnostic() {
        let report = analyze(input(
            "x=1  # pyright: ignore[reportAny]  # sklint: ignore SK401\n",
        ));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK401"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK900"));
    }

    #[test]
    fn noqa_text_inside_docstring_does_not_suppress_docstring_diagnostic() {
        let report = analyze(input(
            r#"def f():
    """
    описание.  # noqa: SK617
    """

    pass
"#,
        ));
        assert!(report.diagnostics.iter().any(|diag| diag.code == "SK617"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK900"));
    }

    #[test]
    fn strict_rule_can_be_enabled_from_inline_config() {
        let report = analyze(input("# sklint: strict\n# TODO: fix me\n"));
        assert!(report.diagnostics.iter().any(|diag| diag.code == "SK101"));
    }

    #[test]
    fn strict_mode_reports_top_level_global_suppressions() {
        let report = analyze(input(
            "# sklint: strict\n# pylint: disable=missing-module-docstring\n# ruff: noqa: F401\n# pyright: reportPrivateUsage=false\n# type: ignore\n# mypy: ignore-errors\n# sklint: ignore=SK804\nvalue = 1\n",
        ));
        let count = report
            .diagnostics
            .iter()
            .filter(|diag| diag.code == "SK805")
            .count();
        assert_eq!(count, 6);
    }

    #[test]
    fn strict_mode_reports_global_suppression_after_module_docstring() {
        let report = analyze(input(
            "# sklint: strict\n\"\"\"Описание модуля.\"\"\"\n# flake8: noqa\nfrom x import y\n",
        ));
        assert!(report.diagnostics.iter().any(|diag| diag.code == "SK805"));
    }

    #[test]
    fn non_strict_mode_does_not_report_global_suppressions() {
        let report = analyze(input(
            "# pylint: disable=missing-module-docstring\nvalue = 1\n",
        ));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK805"));
    }

    #[test]
    fn strict_mode_does_not_report_local_line_suppressions() {
        let report = analyze(input(
            "# sklint: strict\nvalue=1  # noqa: E225\n# локальный комментарий\n# pyright: strict\n",
        ));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK805"));
    }

    #[test]
    fn strict_mode_global_sklint_noqa_cannot_hide_sk805() {
        let report = analyze(input("# sklint: strict\n# sklint: noqa\nvalue = 1\n"));
        assert!(report.diagnostics.iter().any(|diag| diag.code == "SK805"));
    }
    #[test]
    fn sk805_safe_fix_removes_only_forbidden_segment() {
        let report = analyze(input(
            "# sklint: strict\n# ordinary context  # ruff: noqa: F401\nvalue = 1\n",
        ));
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diag| diag.code == "SK805")
            .expect("SK805");
        let fix = diagnostic.fix.as_ref().expect("safe fix");
        assert!(fix.safe);
        assert_eq!(fix.replacement, "# ordinary context");
    }

    #[test]
    fn sk805_safe_fix_preserves_comment_after_suppression() {
        let report = analyze(input(
            "# sklint: strict\n# ruff: noqa: F401  # ordinary context\nvalue = 1\n",
        ));
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diag| diag.code == "SK805")
            .expect("SK805");
        assert_eq!(
            diagnostic.fix.as_ref().expect("safe fix").replacement,
            "# ordinary context"
        );
    }
}
