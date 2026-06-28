use crate::blank_lines::run_blank_line_rules;
use crate::comments::run_comment_rules;
use crate::config::{load_pyproject_for_file, parse_inline_config, EffectiveConfig, VscodeConfig};
use crate::diagnostic::{Diagnostic, Fix, Span};
use crate::docstrings::run_docstring_rules;
use crate::dynamic_attrs::run_dynamic_attribute_rules;
use crate::rules::rule_by_code;
use crate::suppression::SuppressionState;
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
        let suppression_line = diagnostic.suppression_line.unwrap_or(diagnostic.line);
        let ids = suppressions.suppressing_ids_for(suppression_line, &diagnostic.code, None);
        if ids.is_empty() {
            visible.push(diagnostic);
        } else {
            suppressions.mark_hits(&ids);
        }
    }

    if config.is_enabled("SK900") {
        for suppression in suppressions.suppressions.clone() {
            if suppression.hits > 0 {
                continue;
            }
            let ids =
                suppressions.suppressing_ids_for(suppression.line, "SK900", Some(suppression.id));
            if !ids.is_empty() {
                continue;
            }

            let rule = rule_by_code("SK900").expect("SK900 exists");
            visible.push(Diagnostic::new(
                rule.code,
                format!("Unused SKLint suppression `{}`", suppression.text),
                input.path.display().to_string(),
                Span::new(
                    suppression.line,
                    1,
                    suppression.line,
                    suppression.text.chars().count().max(1),
                ),
                "warning",
            ));
        }
    }

    visible.sort_by(|a, b| {
        (a.path.as_str(), a.line, a.column, a.code.as_str()).cmp(&(
            b.path.as_str(),
            b.line,
            b.column,
            b.code.as_str(),
        ))
    });

    AnalysisReport {
        diagnostics: visible,
        config,
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

fn run_rules(path: &Path, source: &str, config: &EffectiveConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let display_path = path.display().to_string();

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
    diagnostics.extend(run_dynamic_attribute_rules(path, source, config));
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
    fn docstring_last_content_line_suppresses_docstring_diagnostic() {
        let report = analyze(input(
            r#"def f():
    """
    описание.  # noqa: SK617
    """

    pass
"#,
        ));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK617"));
        assert!(report.diagnostics.iter().all(|diag| diag.code != "SK900"));
    }

    #[test]
    fn strict_rule_can_be_enabled_from_inline_config() {
        let report = analyze(input("# sklint: strict\n# TODO: fix me\n"));
        assert!(report.diagnostics.iter().any(|diag| diag.code == "SK101"));
    }
}
