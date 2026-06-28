use crate::config::EffectiveConfig;
use crate::diagnostic::{Diagnostic, Fix, Span};
use std::collections::{HashMap, HashSet};
use std::path::Path;

const ALLOWED_SECTIONS: &[&str] = &["Args", "Attributes", "Returns", "Raises"];
const FUNCTION_SECTION_ORDER: &[&str] = &["Args", "Returns", "Raises"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnerKind {
    Module,
    Function,
    Class,
    Constant,
    Other,
}

#[derive(Debug, Clone)]
struct Owner {
    kind: OwnerKind,
    line: usize,
    indent: usize,
    nested: bool,
}

#[derive(Debug, Clone)]
struct Docstring {
    start_line: usize,
    end_line: usize,
    quote: &'static str,
    one_line: bool,
    owner: Owner,
    content: Vec<DocLine>,
}

#[derive(Debug, Clone)]
struct DocLine {
    line: usize,
    text: String,
    start_col: usize,
}

#[derive(Debug, Clone)]
struct Section {
    name: String,
    heading_line: usize,
    start_idx: usize,
    end_idx: usize,
}

#[derive(Debug, Clone)]
struct ClassInfo {
    name: String,
    line: usize,
    bases: Vec<String>,
    has_dataclass: bool,
    doc_start: Option<usize>,
    doc_end: Option<usize>,
    fields: Vec<FieldInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldInfo {
    name: String,
    ty: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttributeDoc {
    name: String,
    ty: Option<String>,
    line: usize,
}

pub fn run_docstring_rules(path: &Path, source: &str, config: &EffectiveConfig) -> Vec<Diagnostic> {
    let display_path = path.display().to_string();
    let lines: Vec<&str> = source.lines().collect();
    let docs = find_docstrings(&lines);
    let classes = collect_classes(&lines, &docs);
    let mut diagnostics = Vec::new();

    for doc in &docs {
        run_rules_for_doc(&display_path, &lines, doc, config, &mut diagnostics);
    }

    run_constant_rules(&display_path, &lines, &docs, config, &mut diagnostics);
    run_dataclass_rules(&display_path, &classes, &docs, config, &mut diagnostics);
    diagnostics
}

fn run_rules_for_doc(
    display_path: &str,
    lines: &[&str],
    doc: &Docstring,
    config: &EffectiveConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if config.is_enabled("SK601") {
        for line_no in doc.start_line..=doc.end_line {
            let line = lines[line_no - 1];
            let clean = line.trim_end_matches([' ', '\t']);
            let len = clean.chars().count();
            if len > 72 {
                diagnostics.push(doc_diag(
                    "SK601",
                    "Docstring line is longer than 72 characters",
                    display_path,
                    Span::new(line_no, 73, line_no, len + 1),
                    doc,
                ));
            }
        }
    }

    if config.is_enabled("SK618") {
        for line_no in doc.start_line..=doc.end_line {
            let line = lines[line_no - 1];
            let clean = line.trim_end_matches([' ', '\t']);
            if clean.len() != line.len() {
                let trailing = &line[clean.len()..];
                if trailing != "  " {
                    let start = clean.chars().count() + 1;
                    let end = line.chars().count() + 1;
                    diagnostics.push(
                        doc_diag(
                            "SK618",
                            "Docstring line has trailing whitespace that is not an intentional two-space Markdown break",
                            display_path,
                            Span::new(line_no, start, line_no, end),
                            doc,
                        )
                        .with_fix(Fix {
                            message: "Remove trailing whitespace in the docstring".to_string(),
                            replacement: String::new(),
                            start_line: line_no,
                            start_column: start,
                            end_line: line_no,
                            end_column: end,
                        }),
                    );
                }
            }
        }
    }

    let sections = sections(doc);

    if config.is_enabled("SK602") {
        for item in &doc.content {
            let trimmed = item.text.trim_start();
            let lower = trimmed.to_ascii_lowercase();
            if lower.starts_with(":param")
                || lower.starts_with(":return")
                || lower.starts_with(":raises")
                || lower.starts_with("@param")
                || lower.starts_with("@return")
                || is_numpy_section_line(trimmed)
            {
                diagnostics.push(doc_diag(
                    "SK602",
                    "Docstring must use Google style instead of reST, Numpy or Javadoc style",
                    display_path,
                    Span::new(
                        item.line,
                        doc_line_first_non_ws_col(item),
                        item.line,
                        doc_line_end_col(item),
                    ),
                    doc,
                ));
            }
        }
    }

    if config.is_enabled("SK606") {
        for item in &doc.content {
            if let Some(name) = top_level_section_heading_name(doc, &item.text) {
                if !ALLOWED_SECTIONS.contains(&name.as_str()) {
                    diagnostics.push(doc_diag(
                        "SK606",
                        format!("Invalid docstring section `{name}`; only Args, Attributes, Returns and Raises are allowed"),
                        display_path,
                        Span::new(item.line, doc_line_first_non_ws_col(item), item.line, doc_line_trimmed_end_col(item)),
                        doc,
                    ));
                }
            }
        }
    }

    if config.is_enabled("SK604") && doc_text_has_letters(doc) && !doc_text_has_cyrillic(doc) {
        diagnostics.push(doc_diag(
            "SK604",
            "Docstring appears to be fully English and does not contain Cyrillic text",
            display_path,
            first_content_span(doc),
            doc,
        ));
    }

    if config.is_enabled("SK603") {
        for (line_no, text) in section_last_lines(doc, &sections) {
            let trimmed = text.trim_end();
            if trimmed.ends_with('.') && final_period_is_sentence_ending(trimmed) {
                let end = doc_line_trimmed_end_col_for(line_no, &text, doc);
                diagnostics.push(
                    doc_diag(
                        "SK603",
                        "The last line of a docstring section must not end with a period",
                        display_path,
                        Span::new(line_no, end - 1, line_no, end),
                        doc,
                    )
                    .with_fix(Fix {
                        message: "Remove the final period from the section".to_string(),
                        replacement: String::new(),
                        start_line: line_no,
                        start_column: end - 1,
                        end_line: line_no,
                        end_column: end,
                    }),
                );
            }
        }
    }

    if config.is_enabled("SK605") && doc.owner.kind == OwnerKind::Function {
        if let Some(line) = first_description_line(doc) {
            if let Some((word, start_col, end_col)) = first_word(&line.text) {
                let start_col = doc_line_col(line, start_col);
                let end_col = doc_line_col(line, end_col);
                if is_action_word(&word) {
                    diagnostics.push(doc_diag(
                        "SK605",
                        "Description must be phrased as a process or state rather than an imperative verb",
                        display_path,
                        Span::new(line.line, start_col, line.line, end_col),
                        doc,
                    ));
                }
            }
        }
    }

    if config.is_enabled("SK607") && doc.owner.nested {
        for (idx, item) in doc.content.iter().enumerate() {
            if idx == 0 || idx + 1 == doc.content.len() {
                continue;
            }
            if item.text.trim().is_empty() {
                diagnostics.push(
                    doc_diag(
                        "SK607",
                        "Nested object docstring must not contain blank lines",
                        display_path,
                        Span::new(
                            item.line,
                            1,
                            item.line,
                            item.text.chars().count().max(1) + 1,
                        ),
                        doc,
                    )
                    .with_fix(Fix {
                        message: "Remove the blank line from the nested docstring".to_string(),
                        replacement: String::new(),
                        start_line: item.line,
                        start_column: 1,
                        end_line: item.line + 1,
                        end_column: 1,
                    }),
                );
            }
        }
    }

    if config.is_enabled("SK608") && doc.owner.nested {
        if let Some(fix) =
            one_line_docstring_fix(doc, lines, "Write the nested docstring on one line")
        {
            diagnostics.push(
                doc_diag(
                    "SK608",
                    "Short nested object docstring must be written on one line",
                    display_path,
                    Span::new(
                        doc.start_line,
                        1,
                        doc.end_line,
                        lines[doc.end_line - 1].chars().count() + 1,
                    ),
                    doc,
                )
                .with_fix(fix),
            );
        }
    }

    if config.is_enabled("SK610") && doc.owner.kind == OwnerKind::Constant {
        if let Some(fix) =
            one_line_docstring_fix(doc, lines, "Write the constant docstring on one line")
        {
            diagnostics.push(
                doc_diag(
                    "SK610",
                    "Short constant docstring must be written on one line",
                    display_path,
                    Span::new(
                        doc.start_line,
                        1,
                        doc.end_line,
                        lines[doc.end_line - 1].chars().count() + 1,
                    ),
                    doc,
                )
                .with_fix(fix),
            );
        }
    }

    if config.is_enabled("SK611") && doc.owner.kind == OwnerKind::Module {
        if let Some(fix) =
            one_line_docstring_fix(doc, lines, "Write the module docstring on one line")
        {
            diagnostics.push(
                doc_diag(
                    "SK611",
                    "Short module docstring must be written on one line",
                    display_path,
                    Span::new(
                        doc.start_line,
                        1,
                        doc.end_line,
                        lines[doc.end_line - 1].chars().count() + 1,
                    ),
                    doc,
                )
                .with_fix(fix),
            );
        }
    }

    if matches!(doc.owner.kind, OwnerKind::Function | OwnerKind::Class) {
        run_object_docstring_rules(display_path, lines, doc, &sections, config, diagnostics);
    }

    if config.is_enabled("SK621") && doc.owner.kind == OwnerKind::Module {
        let mut line_no = doc.end_line + 1;
        while line_no <= lines.len() && lines[line_no - 1].trim().is_empty() {
            diagnostics.push(
                doc_diag(
                    "SK621",
                    "Module docstring must not be followed by blank lines before code",
                    display_path,
                    Span::new(
                        line_no,
                        1,
                        line_no,
                        lines[line_no - 1].chars().count().max(1) + 1,
                    ),
                    doc,
                )
                .with_fix(Fix {
                    message: "Remove the blank line after the module docstring".to_string(),
                    replacement: String::new(),
                    start_line: line_no,
                    start_column: 1,
                    end_line: (line_no + 1).min(lines.len() + 1),
                    end_column: 1,
                }),
            );
            line_no += 1;
        }
    }

    if config.is_enabled("SK624") {
        run_item_continuation_rule(display_path, doc, &sections, diagnostics);
    }
}

fn is_stub_function_docstring(lines: &[&str], doc: &Docstring) -> bool {
    if doc.owner.kind != OwnerKind::Function {
        return false;
    }
    let body_indent = doc.owner.indent + 4;
    for line_no in doc.end_line + 1..=lines.len() {
        let line = lines[line_no - 1];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if indent_width(line) <= doc.owner.indent {
            return false;
        }
        return indent_width(line) == body_indent && trimmed == "...";
    }
    false
}

fn is_line_inside_docstring_owner_body(line: &str, doc: &Docstring) -> bool {
    matches!(doc.owner.kind, OwnerKind::Function | OwnerKind::Class)
        && indent_width(line) > doc.owner.indent
}

fn run_object_docstring_rules(
    display_path: &str,
    lines: &[&str],
    doc: &Docstring,
    sections: &[Section],
    config: &EffectiveConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if config.is_enabled("SK612") && !doc.owner.nested {
        let start_line = lines[doc.start_line - 1];
        let end_line = lines[doc.end_line - 1];
        let start_after_quote = start_line
            .find(doc.quote)
            .map(|idx| &start_line[idx + 3..])
            .unwrap_or_default();
        let end_before_quote = end_line
            .rfind(doc.quote)
            .map(|idx| &end_line[..idx])
            .unwrap_or_default();
        if doc.one_line
            || !start_after_quote.trim().is_empty()
            || !end_before_quote.trim().is_empty()
        {
            let mut diagnostic = doc_diag(
                "SK612",
                "Non-nested function, method and class docstring quotes must be on separate lines",
                display_path,
                Span::new(
                    doc.start_line,
                    first_non_ws_col(start_line),
                    doc.end_line,
                    end_line.chars().count() + 1,
                ),
                doc,
            );
            if doc.one_line {
                let indent = " ".repeat(indent_width(start_line));
                let content = doc
                    .content
                    .first()
                    .map(|line| line.text.trim())
                    .unwrap_or("");
                diagnostic = diagnostic.with_fix(Fix {
                    message: "Move docstring quotes to separate lines".to_string(),
                    replacement: format!(
                        "{indent}{q}
{indent}{content}
{indent}{q}",
                        q = doc.quote
                    ),
                    start_line: doc.start_line,
                    start_column: 1,
                    end_line: doc.end_line,
                    end_column: end_line.chars().count() + 1,
                });
            } else if !start_after_quote.trim().is_empty() && end_before_quote.trim().is_empty() {
                let indent = " ".repeat(indent_width(start_line));
                let content = start_after_quote.trim_end();
                diagnostic = diagnostic.with_fix(Fix {
                    message: "Move opening docstring quotes to a separate line".to_string(),
                    replacement: format!(
                        "{indent}{q}
{indent}{content}",
                        q = doc.quote
                    ),
                    start_line: doc.start_line,
                    start_column: 1,
                    end_line: doc.start_line,
                    end_column: start_line.chars().count() + 1,
                });
            }
            diagnostics.push(diagnostic);
        }
    }

    if config.is_enabled("SK613") && !doc.one_line && !is_stub_function_docstring(lines, doc) {
        let next = doc.end_line + 1;
        if next <= lines.len()
            && !lines[next - 1].trim().is_empty()
            && is_line_inside_docstring_owner_body(lines[next - 1], doc)
        {
            let insert_col = lines[doc.end_line - 1].chars().count() + 1;
            diagnostics.push(
                doc_diag(
                    "SK613",
                    "Multiline function, method and class docstrings must be followed by a blank line",
                    display_path,
                    Span::new(next, 1, next, lines[next - 1].chars().count().max(1) + 1),
                    doc,
                )
                .with_fix(Fix {
                    message: "Add a blank line after the docstring".to_string(),
                    replacement: "\n".to_string(),
                    start_line: doc.end_line,
                    start_column: insert_col,
                    end_line: doc.end_line,
                    end_column: insert_col,
                }),
            );
        }
    }

    if config.is_enabled("SK614") && description_lines_before_sections(doc).is_empty() {
        let insert_line = if doc.one_line {
            doc.start_line
        } else {
            doc.start_line + 1
        };
        let indent = " ".repeat(owner_body_indent(doc));
        diagnostics.push(
            doc_diag(
                "SK614",
                "Function, method and class docstrings must contain a description outside Args, Attributes, Returns and Raises",
                display_path,
                Span::new(doc.start_line, first_non_ws_col(lines[doc.start_line - 1]), doc.start_line, lines[doc.start_line - 1].chars().count() + 1),
                doc,
            )
            .with_fix(Fix {
                message: "Add a docstring description template".to_string(),
                replacement: format!("{indent}TODO: описание\n"),
                start_line: insert_line,
                start_column: 1,
                end_line: insert_line,
                end_column: 1,
            }),
        );
    }

    if config.is_enabled("SK615") && !doc.owner.nested {
        if let Some(first_section) = sections.first() {
            let description = description_lines_before_sections(doc);
            if !description.is_empty() {
                let prev_idx = first_section.start_idx.saturating_sub(1);
                if prev_idx < doc.content.len() && !doc.content[prev_idx].text.trim().is_empty() {
                    diagnostics.push(
                        doc_diag(
                            "SK615",
                            "A blank docstring line is required between the description and structured sections",
                            display_path,
                            Span::new(first_section.heading_line, 1, first_section.heading_line, doc.content[first_section.start_idx].text.chars().count() + 1),
                            doc,
                        )
                        .with_fix(Fix {
                            message: "Add a blank line before structured sections".to_string(),
                            replacement: "\n".to_string(),
                            start_line: first_section.heading_line,
                            start_column: 1,
                            end_line: first_section.heading_line,
                            end_column: 1,
                        }),
                    );
                }
            }
        }
    }

    if config.is_enabled("SK616") {
        if let Some(line) = first_description_line(doc) {
            if let Some((word, start_col, end_col)) = first_word(&line.text) {
                let start_col = doc_line_col(line, start_col);
                let end_col = doc_line_col(line, end_col);
                let lower = word.to_lowercase();
                let expected = match doc.owner.kind {
                    OwnerKind::Function if lower == "функция" || lower == "метод" => {
                        true
                    }
                    OwnerKind::Class if lower == "класс" => true,
                    _ => false,
                };
                if expected {
                    diagnostics.push(
                        doc_diag(
                            "SK616",
                            format!("Description must not start with the redundant word `{word}`"),
                            display_path,
                            Span::new(line.line, start_col, line.line, end_col),
                            doc,
                        )
                        .with_fix(Fix {
                            message: format!("Remove the redundant word `{word}`"),
                            replacement: String::new(),
                            start_line: line.line,
                            start_column: start_col,
                            end_line: line.line,
                            end_column: (end_col + 1).min(doc_line_end_col(line)),
                        }),
                    );
                }
            }
        }
    }

    if config.is_enabled("SK617") {
        run_capitalization_rule(display_path, doc, diagnostics);
    }

    if matches!(doc.owner.kind, OwnerKind::Function) {
        if config.is_enabled("SK622") {
            run_no_blank_between_function_sections(display_path, doc, sections, diagnostics);
        }
        if config.is_enabled("SK623") {
            run_function_section_order(display_path, doc, sections, diagnostics);
        }
    }
}

fn run_capitalization_rule(display_path: &str, doc: &Docstring, diagnostics: &mut Vec<Diagnostic>) {
    let mut at_sentence_start = true;

    for line in &doc.content {
        let trimmed = line.text.trim();
        if trimmed.is_empty() {
            at_sentence_start = true;
            continue;
        }

        if section_heading_name(&line.text).is_some() {
            at_sentence_start = true;
            continue;
        }

        for (idx, ch) in line.text.char_indices() {
            if ch.is_whitespace() {
                continue;
            }
            if at_sentence_start && is_cyrillic_lower(ch) {
                let upper = ch.to_uppercase().collect::<String>();
                let col = doc_line_col(line, line.text[..idx].chars().count() + 1);
                diagnostics.push(
                    doc_diag(
                        "SK617",
                        "Cyrillic docstring sentences must start with an uppercase letter",
                        display_path,
                        Span::new(line.line, col, line.line, col + 1),
                        doc,
                    )
                    .with_fix(Fix {
                        message: "Uppercase the first letter of the sentence".to_string(),
                        replacement: upper,
                        start_line: line.line,
                        start_column: col,
                        end_line: line.line,
                        end_column: col + 1,
                    }),
                );
                break;
            }

            at_sentence_start = matches!(ch, '!' | '?')
                || (ch == '.' && period_is_sentence_ending(&line.text, idx));
        }
    }
}

fn run_no_blank_between_function_sections(
    display_path: &str,
    doc: &Docstring,
    sections: &[Section],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let function_sections: Vec<&Section> = sections
        .iter()
        .filter(|section| FUNCTION_SECTION_ORDER.contains(&section.name.as_str()))
        .collect();

    for pair in function_sections.windows(2) {
        let prev = pair[0];
        let next = pair[1];
        let mut blank_lines = Vec::new();
        for idx in prev.end_idx + 1..next.start_idx {
            if doc.content[idx].text.trim().is_empty() {
                blank_lines.push(doc.content[idx].line);
            }
        }
        for line_no in blank_lines {
            diagnostics.push(
                doc_diag(
                    "SK622",
                    "Args, Returns and Raises sections must not be separated by blank lines",
                    display_path,
                    Span::new(line_no, 1, line_no, 1),
                    doc,
                )
                .with_fix(Fix {
                    message: "Remove the blank line between sections".to_string(),
                    replacement: String::new(),
                    start_line: line_no,
                    start_column: 1,
                    end_line: line_no + 1,
                    end_column: 1,
                }),
            );
        }
    }
}

fn run_function_section_order(
    display_path: &str,
    doc: &Docstring,
    sections: &[Section],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let function_sections: Vec<&Section> = sections
        .iter()
        .filter(|section| FUNCTION_SECTION_ORDER.contains(&section.name.as_str()))
        .collect();
    let mut max_order = 0usize;
    let mut out_of_order = false;
    for section in &function_sections {
        let Some(order) = FUNCTION_SECTION_ORDER
            .iter()
            .position(|name| *name == section.name)
        else {
            continue;
        };
        if order < max_order {
            out_of_order = true;
            break;
        }
        max_order = max_order.max(order);
    }
    if !out_of_order {
        return;
    }

    let Some(first) = function_sections.first() else {
        return;
    };
    let Some(last) = function_sections.last() else {
        return;
    };

    let mut ordered = function_sections.clone();
    ordered.sort_by_key(|section| {
        FUNCTION_SECTION_ORDER
            .iter()
            .position(|name| *name == section.name)
            .unwrap_or(usize::MAX)
    });

    let mut lines = Vec::new();
    for (pos, section) in ordered.iter().enumerate() {
        if pos > 0 {
            while lines
                .last()
                .is_some_and(|line: &String| line.trim().is_empty())
            {
                lines.pop();
            }
        }
        for idx in section.start_idx..=section.end_idx {
            lines.push(doc.content[idx].text.clone());
        }
    }

    let replacement = lines.join("\n");
    let end_line = doc.content[last.end_idx].line;
    let end_column = doc.content[last.end_idx].text.chars().count() + 1;
    diagnostics.push(
        doc_diag(
            "SK623",
            "Function and method sections must appear in Args, Returns, Raises order",
            display_path,
            Span::new(
                first.heading_line,
                doc_line_first_non_ws_col(&doc.content[first.start_idx]),
                first.heading_line,
                doc_line_trimmed_end_col(&doc.content[first.start_idx]),
            ),
            doc,
        )
        .with_fix(Fix {
            message: "Reorder function docstring sections".to_string(),
            replacement,
            start_line: doc.content[first.start_idx].line,
            start_column: 1,
            end_line,
            end_column,
        }),
    );
}

fn run_item_continuation_rule(
    display_path: &str,
    doc: &Docstring,
    sections: &[Section],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for section in sections {
        if !matches!(section.name.as_str(), "Args" | "Attributes" | "Raises") {
            continue;
        }
        for idx in section.start_idx + 1..=section.end_idx {
            let item = &doc.content[idx];
            if item.text.trim().is_empty() || section_heading_name(&item.text).is_some() {
                continue;
            }
            let clean = item.text.trim_end_matches([' ', '\t']);
            if clean.chars().count() <= 72 {
                continue;
            }
            let Some(colon_idx) = item.text.find(':') else {
                continue;
            };
            let after_colon = item.text[colon_idx + 1..].trim_start();
            if after_colon.is_empty() {
                continue;
            }
            let prefix = &item.text[..=colon_idx];
            let indent = item
                .text
                .chars()
                .take_while(|ch| ch.is_whitespace())
                .count();
            let replacement = format!(
                "{}\n{}{}",
                prefix.trim_end(),
                " ".repeat(indent + 4),
                after_colon.trim_end()
            );
            diagnostics.push(
                doc_diag(
                    "SK624",
                    "Long argument, attribute or exception description must start on the next indented line",
                    display_path,
                    Span::new(item.line, 73, item.line, clean.chars().count() + 1),
                    doc,
                )
                .with_fix(Fix {
                    message: "Move the long item description to the next line".to_string(),
                    replacement,
                    start_line: item.line,
                    start_column: 1,
                    end_line: item.line,
                    end_column: item.text.chars().count() + 1,
                }),
            );
        }
    }
}

fn run_constant_rules(
    display_path: &str,
    lines: &[&str],
    docs: &[Docstring],
    config: &EffectiveConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !config.is_enabled("SK609") {
        return;
    }

    for (idx, line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        if docs
            .iter()
            .any(|doc| doc.start_line <= line_no && line_no <= doc.end_line)
        {
            continue;
        }
        if !is_final_assignment(line)
            || final_assignment_name(line).is_some_and(|name| name == "__all__")
        {
            continue;
        }
        let assignment_end_line = final_assignment_end_line(lines, idx);
        let Some(next_code_line) = next_non_empty_line(lines, assignment_end_line + 1) else {
            diagnostics.push(missing_constant_doc_diag(
                display_path,
                line_no,
                assignment_end_line,
                lines[assignment_end_line - 1].chars().count() + 1,
                constant_doc_indent(lines[idx]),
            ));
            continue;
        };
        let has_doc = docs.iter().any(|doc| {
            doc.owner.kind == OwnerKind::Constant
                && doc.owner.line == line_no
                && doc.start_line == next_code_line
        });
        if !has_doc {
            diagnostics.push(missing_constant_doc_diag(
                display_path,
                line_no,
                assignment_end_line,
                lines[assignment_end_line - 1].chars().count() + 1,
                constant_doc_indent(lines[idx]),
            ));
        }
    }
}

fn constant_doc_indent(line: &str) -> String {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect()
}

fn missing_constant_doc_diag(
    display_path: &str,
    line_no: usize,
    assignment_end_line: usize,
    insert_col: usize,
    indent: String,
) -> Diagnostic {
    Diagnostic::new(
        "SK609",
        "Final constant must have an immediate string docstring description",
        display_path,
        Span::new(line_no, 1, assignment_end_line + 1, insert_col),
        "warning",
    )
    .with_fix(Fix {
        message: "Create a constant docstring template".to_string(),
        replacement: format!("\n{indent}\"\"\"TODO: описание константы\"\"\""),
        start_line: assignment_end_line,
        start_column: insert_col,
        end_line: assignment_end_line,
        end_column: insert_col,
    })
}

fn run_dataclass_rules(
    display_path: &str,
    classes: &[ClassInfo],
    docs: &[Docstring],
    config: &EffectiveConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !config.is_enabled("SK619") && !config.is_enabled("SK620") {
        return;
    }

    let by_name: HashMap<String, &ClassInfo> = classes
        .iter()
        .map(|class| (class.name.clone(), class))
        .collect();
    for class in classes.iter().filter(|class| class.has_dataclass) {
        let Some(doc_start) = class.doc_start else {
            continue;
        };
        let Some(doc) = docs.iter().find(|doc| doc.start_line == doc_start) else {
            continue;
        };
        let fields = inherited_fields(class, &by_name);
        if fields.is_empty() {
            continue;
        }
        let attrs = attribute_docs(doc);
        let attrs_by_name: HashMap<String, AttributeDoc> = attrs
            .iter()
            .cloned()
            .map(|attr| (attr.name.clone(), attr))
            .collect();
        let documented_order: Vec<String> = attrs.iter().map(|attr| attr.name.clone()).collect();
        let expected_order: Vec<String> = fields.iter().map(|field| field.name.clone()).collect();
        let missing: Vec<&FieldInfo> = fields
            .iter()
            .filter(|field| !attrs_by_name.contains_key(&field.name))
            .collect();
        let order_mismatch = !documented_order.is_empty()
            && documented_order
                .iter()
                .filter(|name| expected_order.contains(name))
                .ne(expected_order
                    .iter()
                    .filter(|name| documented_order.contains(name)));

        let attr_fix = dataclass_attributes_fix(doc, &fields, &attrs_by_name);

        if config.is_enabled("SK619") && (!missing.is_empty() || order_mismatch) {
            let message = if missing.is_empty() {
                "Attributes section order does not match dataclass field order".to_string()
            } else {
                format!(
                    "Attributes section does not document dataclass fields: {}",
                    missing
                        .iter()
                        .map(|field| field.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let line = class.doc_end.unwrap_or(class.line);
            let mut diagnostic = doc_diag(
                "SK619",
                message,
                display_path,
                Span::new(line, 1, line, 1),
                doc,
            );
            if let Some(fix) = attr_fix.clone() {
                diagnostic = diagnostic.with_fix(fix);
            }
            diagnostics.push(diagnostic);
        }

        if config.is_enabled("SK620") {
            let mut mismatches = Vec::new();
            for field in &fields {
                let Some(attr) = attrs_by_name.get(&field.name) else {
                    continue;
                };
                let Some(doc_ty) = &attr.ty else {
                    continue;
                };
                if normalize_type(doc_ty) != normalize_type(&field.ty) {
                    mismatches.push((field, attr, doc_ty.clone()));
                }
            }
            if let Some((field, attr, doc_ty)) = mismatches.first() {
                let attr_text = doc
                    .content
                    .iter()
                    .find(|line| line.line == attr.line)
                    .map(|line| line.text.as_str())
                    .unwrap_or("");
                let mut diagnostic = doc_diag(
                    "SK620",
                    format!(
                        "Attributes type for `{}` (`{doc_ty}`) does not match field annotation `{}`",
                        field.name, field.ty
                    ),
                    display_path,
                    Span::new(
                        attr.line,
                        first_non_ws_col(attr_text),
                        attr.line,
                        attr_text.chars().count().max(1) + 1,
                    ),
                    doc,
                );
                if let Some(fix) = attr_fix {
                    diagnostic = diagnostic.with_fix(fix);
                }
                diagnostics.push(diagnostic);
            }
        }
    }
}

fn dataclass_attributes_fix(
    doc: &Docstring,
    fields: &[FieldInfo],
    attrs_by_name: &HashMap<String, AttributeDoc>,
) -> Option<Fix> {
    let attr_section = sections(doc)
        .into_iter()
        .find(|section| section.name == "Attributes");
    let indent = " ".repeat(owner_body_indent(doc));
    let item_indent = format!("{indent}    ");
    let mut replacement_lines = Vec::new();
    replacement_lines.push(format!("{indent}Attributes:"));
    for field in fields {
        let description = attrs_by_name
            .get(&field.name)
            .and_then(|attr| attribute_description(doc, attr.line))
            .unwrap_or_else(|| "TODO: описание".to_string());
        replacement_lines.push(format!(
            "{item_indent}{} ({}): {}",
            field.name, field.ty, description
        ));
    }
    let replacement = replacement_lines.join("\n");

    if let Some(section) = attr_section {
        let start_line = doc.content[section.start_idx].line;
        let mut end_idx = section.end_idx;
        while end_idx > section.start_idx && doc.content[end_idx].text.trim().is_empty() {
            end_idx -= 1;
        }
        let end_line = doc.content[end_idx].line;
        let end_column = doc.content[end_idx].text.chars().count() + 1;
        return Some(Fix {
            message: "Rewrite dataclass Attributes section in field order".to_string(),
            replacement,
            start_line,
            start_column: 1,
            end_line,
            end_column,
        });
    }

    let insert_line = doc.end_line;
    Some(Fix {
        message: "Create dataclass Attributes section template".to_string(),
        replacement: format!("\n{replacement}"),
        start_line: insert_line,
        start_column: 1,
        end_line: insert_line,
        end_column: 1,
    })
}

fn attribute_description(doc: &Docstring, line_no: usize) -> Option<String> {
    let line = doc.content.iter().find(|line| line.line == line_no)?;
    let (_head, desc) = line.text.trim().split_once(':')?;
    let desc = desc.trim();
    if desc.is_empty() {
        None
    } else {
        Some(desc.to_string())
    }
}

fn find_docstrings(lines: &[&str]) -> Vec<Docstring> {
    let mut docs = Vec::new();
    let mut idx = 0usize;
    while idx < lines.len() {
        let line = lines[idx];
        let quote_pos = find_quote(line);
        let Some((start_col_byte, quote)) = quote_pos else {
            idx += 1;
            continue;
        };

        if !looks_like_docstring_start(lines, idx, start_col_byte) {
            idx += 1;
            continue;
        }

        let after = &line[start_col_byte + 3..];
        let owner = owner_for_docstring(lines, idx + 1);
        if let Some(end_rel) = after.find(quote) {
            let content_text = after[..end_rel].to_string();
            docs.push(Docstring {
                start_line: idx + 1,
                end_line: idx + 1,
                quote,
                one_line: true,
                owner,
                content: vec![DocLine {
                    line: idx + 1,
                    text: content_text,
                    start_col: line[..start_col_byte + 3].chars().count() + 1,
                }],
            });
            idx += 1;
            continue;
        }

        let mut content = Vec::new();
        let first_text = after.to_string();
        content.push(DocLine {
            line: idx + 1,
            text: first_text,
            start_col: line[..start_col_byte + 3].chars().count() + 1,
        });

        let mut end_idx = idx;
        let mut found = false;
        for (scan_idx, scan_line) in lines.iter().enumerate().skip(idx + 1) {
            if let Some(end_pos) = scan_line.find(quote) {
                let before = scan_line[..end_pos].to_string();
                content.push(DocLine {
                    line: scan_idx + 1,
                    text: before,
                    start_col: 1,
                });
                end_idx = scan_idx;
                found = true;
                break;
            }
            content.push(DocLine {
                line: scan_idx + 1,
                text: (*scan_line).to_string(),
                start_col: 1,
            });
        }

        if found {
            docs.push(Docstring {
                start_line: idx + 1,
                end_line: end_idx + 1,
                quote,
                one_line: false,
                owner,
                content,
            });
            idx = end_idx + 1;
        } else {
            idx += 1;
        }
    }
    docs
}

fn find_quote(line: &str) -> Option<(usize, &'static str)> {
    let double = line.find("\"\"\"");
    let single = line.find("'''");
    match (double, single) {
        (Some(d), Some(s)) if d < s => Some((d, "\"\"\"")),
        (Some(_), Some(s)) => Some((s, "'''")),
        (Some(d), None) => Some((d, "\"\"\"")),
        (None, Some(s)) => Some((s, "'''")),
        (None, None) => None,
    }
}

fn looks_like_docstring_start(lines: &[&str], idx: usize, quote_byte: usize) -> bool {
    if !lines[idx][..quote_byte].trim().is_empty() {
        return false;
    }
    let owner = owner_for_docstring(lines, idx + 1);
    owner.kind != OwnerKind::Other
}

fn owner_for_docstring(lines: &[&str], doc_line: usize) -> Owner {
    let idx = doc_line - 1;
    if is_module_docstring(lines, idx) {
        return Owner {
            kind: OwnerKind::Module,
            line: doc_line,
            indent: 0,
            nested: false,
        };
    }

    let mut previous = idx;
    while previous > 0 {
        previous -= 1;
        let text = lines[previous].trim();
        if text.is_empty() || text.starts_with('#') || text.starts_with('@') {
            continue;
        }
        let raw = lines[previous];
        let indent = indent_width(raw);
        if parse_def_name(text).is_some() {
            return Owner {
                kind: OwnerKind::Function,
                line: previous + 1,
                indent,
                nested: is_nested_owner(lines, previous),
            };
        }
        if parse_class_header(text).is_some() {
            return Owner {
                kind: OwnerKind::Class,
                line: previous + 1,
                indent,
                nested: is_nested_owner(lines, previous),
            };
        }
        if let Some(owner) = multiline_owner_for_docstring(lines, previous) {
            return owner;
        }
        if is_final_assignment(raw) {
            return Owner {
                kind: OwnerKind::Constant,
                line: previous + 1,
                indent,
                nested: false,
            };
        }
        if let Some(start_idx) = multiline_final_assignment_start(lines, previous) {
            return Owner {
                kind: OwnerKind::Constant,
                line: start_idx + 1,
                indent: indent_width(lines[start_idx]),
                nested: false,
            };
        }
        break;
    }

    Owner {
        kind: OwnerKind::Other,
        line: doc_line,
        indent: 0,
        nested: false,
    }
}

fn multiline_owner_for_docstring(lines: &[&str], header_end_idx: usize) -> Option<Owner> {
    for start_idx in (0..=header_end_idx).rev() {
        let text = lines[start_idx].trim_start();
        let indent = indent_width(lines[start_idx]);
        if parse_def_name(text).is_some() {
            if doc_header_end_line(lines, start_idx) == header_end_idx + 1 {
                return Some(Owner {
                    kind: OwnerKind::Function,
                    line: start_idx + 1,
                    indent,
                    nested: is_nested_owner(lines, start_idx),
                });
            }
            return None;
        }
        if parse_class_header(text).is_some() {
            if doc_header_end_line(lines, start_idx) == header_end_idx + 1 {
                return Some(Owner {
                    kind: OwnerKind::Class,
                    line: start_idx + 1,
                    indent,
                    nested: is_nested_owner(lines, start_idx),
                });
            }
            return None;
        }
        if text.is_empty() || text.starts_with('@') || text.starts_with('#') {
            continue;
        }
    }
    None
}

fn doc_header_end_line(lines: &[&str], start_idx: usize) -> usize {
    let mut depth = 0usize;
    for (idx, line) in lines.iter().enumerate().skip(start_idx) {
        let code = strip_doc_line_comment_and_strings(line);
        for ch in code.chars() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        if depth == 0 && code.trim_end().ends_with(':') {
            return idx + 1;
        }
    }
    start_idx + 1
}

fn strip_doc_line_comment_and_strings(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut quote: Option<char> = None;
    let mut escape = false;
    for ch in line.chars() {
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
            continue;
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

fn is_module_docstring(lines: &[&str], idx: usize) -> bool {
    for line in &lines[..idx] {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("from __future__ import") {
            continue;
        }
        return false;
    }
    true
}

fn is_nested_owner(lines: &[&str], owner_idx: usize) -> bool {
    let owner_indent = indent_width(lines[owner_idx]);
    let mut saw_class_at_parent = false;
    for line in lines[..owner_idx].iter().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@') {
            continue;
        }
        let indent = indent_width(line);
        if indent >= owner_indent {
            continue;
        }
        if parse_def_name(trimmed).is_some() {
            return true;
        }
        if parse_class_header(trimmed).is_some() {
            saw_class_at_parent = true;
            continue;
        }
        if indent == 0 {
            break;
        }
    }
    // A method directly inside a class is not treated as a nested object for
    // one-line nested-docstring rules. Inner classes/functions still are.
    if saw_class_at_parent && owner_indent == 4 {
        return false;
    }
    owner_indent > 4
}

fn sections(doc: &Docstring) -> Vec<Section> {
    let mut starts = Vec::new();
    for (idx, line) in doc.content.iter().enumerate() {
        if let Some(name) = top_level_section_heading_name(doc, &line.text) {
            starts.push((idx, name));
        }
    }

    let mut sections = Vec::new();
    for (pos, (start_idx, name)) in starts.iter().enumerate() {
        let end_idx = starts
            .get(pos + 1)
            .map(|(next, _)| next.saturating_sub(1))
            .unwrap_or_else(|| doc.content.len().saturating_sub(1));
        sections.push(Section {
            name: name.clone(),
            heading_line: doc.content[*start_idx].line,
            start_idx: *start_idx,
            end_idx,
        });
    }
    sections
}

fn section_heading_name(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let name = trimmed.strip_suffix(':')?;
    if name.is_empty() || name.contains('(') || name.contains(')') || name.contains(',') {
        return None;
    }
    if name.chars().any(|ch| ch.is_ascii_digit()) {
        return None;
    }
    if name.split_whitespace().count() > 3 {
        return None;
    }
    let first = name.chars().next()?;
    if !first.is_uppercase() && !first.is_ascii_uppercase() {
        return None;
    }
    Some(name.to_string())
}

fn top_level_section_heading_name(doc: &Docstring, text: &str) -> Option<String> {
    if indent_width(text) > owner_body_indent(doc) {
        return None;
    }
    section_heading_name(text)
}

fn is_numpy_section_line(text: &str) -> bool {
    matches!(text, "Parameters" | "Returns" | "Raises" | "Attributes")
}

fn doc_text_has_letters(doc: &Docstring) -> bool {
    doc.content
        .iter()
        .any(|line| line.text.chars().any(char::is_alphabetic))
}

fn doc_text_has_cyrillic(doc: &Docstring) -> bool {
    doc.content
        .iter()
        .any(|line| line.text.chars().any(is_cyrillic))
}

fn is_cyrillic(ch: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&ch) || ('\u{0500}'..='\u{052F}').contains(&ch)
}

fn is_cyrillic_lower(ch: char) -> bool {
    is_cyrillic(ch) && ch.is_lowercase()
}

fn final_period_is_sentence_ending(trimmed: &str) -> bool {
    let Some((byte_idx, _)) = trimmed.char_indices().next_back() else {
        return false;
    };
    period_is_sentence_ending(trimmed, byte_idx)
}

fn period_is_sentence_ending(text: &str, byte_idx: usize) -> bool {
    if is_known_abbreviation_period(text, byte_idx) {
        return false;
    }
    text[..byte_idx]
        .chars()
        .rev()
        .find(|ch| !ch.is_whitespace())
        .is_none_or(|ch| !ch.is_ascii_digit())
}

fn is_known_abbreviation_period(text: &str, byte_idx: usize) -> bool {
    const ABBREVIATIONS: &[&str] = &[
        "т.д.",
        "т.п.",
        "т.е.",
        "т.к.",
        "т.н.",
        "ит.д.",
        "ит.п.",
        "ит.е.",
        "др.",
        "см.",
        "пр.",
        "г.",
        "гг.",
        "стр.",
        "рис.",
        "табл.",
        "им.",
        "ул.",
    ];

    let mut chars = Vec::new();
    let mut byte_map = Vec::new();
    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            continue;
        }
        for lower in ch.to_lowercase() {
            chars.push(lower);
            byte_map.push(idx);
        }
    }

    let Some(norm_idx) = byte_map.iter().position(|idx| *idx == byte_idx) else {
        return false;
    };
    if chars.get(norm_idx) != Some(&'.') {
        return false;
    }

    ABBREVIATIONS.iter().any(|abbr| {
        let abbr_chars = abbr.chars().collect::<Vec<_>>();
        chars
            .windows(abbr_chars.len())
            .enumerate()
            .any(|(start, window)| {
                window == abbr_chars.as_slice()
                    && (start..start + abbr_chars.len()).contains(&norm_idx)
            })
    })
}

fn first_content_span(doc: &Docstring) -> Span {
    for line in &doc.content {
        if !line.text.trim().is_empty() {
            return Span::new(
                line.line,
                doc_line_first_non_ws_col(line),
                line.line,
                doc_line_trimmed_end_col(line),
            );
        }
    }
    Span::new(doc.start_line, 1, doc.start_line, 1)
}

fn section_last_lines(doc: &Docstring, sections: &[Section]) -> Vec<(usize, String)> {
    let mut ranges = Vec::new();
    if let Some(first_section) = sections.first() {
        if first_section.start_idx > 0 {
            ranges.push((0, first_section.start_idx - 1));
        }
    } else if !doc.content.is_empty() {
        ranges.push((0, doc.content.len() - 1));
    }
    for section in sections {
        ranges.push((section.start_idx + 1, section.end_idx));
    }

    let mut out = Vec::new();
    for (start, end) in ranges {
        if start > end || end >= doc.content.len() {
            continue;
        }
        for idx in (start..=end).rev() {
            let line = &doc.content[idx];
            if !line.text.trim().is_empty()
                && top_level_section_heading_name(doc, &line.text).is_none()
            {
                out.push((line.line, line.text.clone()));
                break;
            }
        }
    }
    out
}

fn first_description_line(doc: &Docstring) -> Option<&DocLine> {
    doc.content
        .iter()
        .take_while(|line| top_level_section_heading_name(doc, &line.text).is_none())
        .find(|line| !line.text.trim().is_empty())
}

fn description_lines_before_sections(doc: &Docstring) -> Vec<&DocLine> {
    doc.content
        .iter()
        .take_while(|line| top_level_section_heading_name(doc, &line.text).is_none())
        .filter(|line| !line.text.trim().is_empty())
        .collect()
}

fn first_word(text: &str) -> Option<(String, usize, usize)> {
    let start_byte = text.find(|ch: char| ch.is_alphabetic())?;
    let mut end_byte = text.len();
    for (idx, ch) in text[start_byte..].char_indices() {
        if !(ch.is_alphabetic() || ch == '-') {
            end_byte = start_byte + idx;
            break;
        }
    }
    let word = text[start_byte..end_byte].to_string();
    let start_col = text[..start_byte].chars().count() + 1;
    let end_col = start_col + word.chars().count();
    Some((word, start_col, end_col))
}

fn is_action_word(word: &str) -> bool {
    let lower = word.to_lowercase();
    lower.ends_with("ть")
        || lower.ends_with("ться")
        || lower.ends_with("ти")
        || lower.ends_with("чь")
}

fn one_line_docstring_fix(doc: &Docstring, lines: &[&str], message: &str) -> Option<Fix> {
    if doc.one_line || doc.content.len() != 3 {
        return None;
    }
    let middle = doc.content.get(1)?.text.trim();
    if middle.is_empty() || doc.content[0].text.trim() != "" || doc.content[2].text.trim() != "" {
        return None;
    }
    let indent = " ".repeat(indent_width(lines[doc.start_line - 1]));
    let replacement = format!("{indent}{q}{middle}{q}", q = doc.quote);
    if replacement.chars().count() > 72 {
        return None;
    }
    Some(Fix {
        message: message.to_string(),
        replacement,
        start_line: doc.start_line,
        start_column: 1,
        end_line: doc.end_line,
        end_column: lines[doc.end_line - 1].chars().count() + 1,
    })
}

fn owner_body_indent(doc: &Docstring) -> usize {
    doc.owner.indent + 4
}

fn next_non_empty_line(lines: &[&str], start_line: usize) -> Option<usize> {
    (start_line..=lines.len()).find(|&line_no| !lines[line_no - 1].trim().is_empty())
}

fn final_assignment_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let (name, _) = trimmed.split_once(':')?;
    let name = name.trim();
    if name.is_empty() || name.contains([' ', '.', '[', ']']) {
        None
    } else {
        Some(name)
    }
}

fn is_final_assignment(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with("def ") || trimmed.starts_with("class ") {
        return false;
    }
    trimmed.contains(": Final") || trimmed.contains(":Final")
}

fn multiline_final_assignment_start(lines: &[&str], end_idx: usize) -> Option<usize> {
    for start_idx in (0..=end_idx).rev() {
        if !is_final_assignment(lines[start_idx]) {
            continue;
        }
        if final_assignment_end_line(lines, start_idx) == end_idx + 1 {
            return Some(start_idx);
        }
    }
    None
}

fn final_assignment_end_line(lines: &[&str], start_idx: usize) -> usize {
    let mut depth = 0isize;
    let mut saw_open = false;
    let mut quote: Option<char> = None;
    let mut triple_quote: Option<char> = None;
    let mut escaped = false;

    for (idx, line) in lines.iter().enumerate().skip(start_idx) {
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let mut pos = 0usize;
        while pos < chars.len() {
            let (byte_idx, ch) = chars[pos];

            if let Some(active) = triple_quote {
                if ch == active && line[byte_idx..].starts_with(&active.to_string().repeat(3)) {
                    triple_quote = None;
                    pos += 3;
                } else {
                    pos += 1;
                }
                continue;
            }

            if let Some(active) = quote {
                if escaped {
                    escaped = false;
                    pos += 1;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                    pos += 1;
                    continue;
                }
                if ch == active {
                    quote = None;
                }
                pos += 1;
                continue;
            }

            match ch {
                '#' => break,
                '\'' | '"' => {
                    if line[byte_idx..].starts_with(&ch.to_string().repeat(3)) {
                        triple_quote = Some(ch);
                        pos += 3;
                    } else {
                        quote = Some(ch);
                        pos += 1;
                    }
                }
                '(' | '[' | '{' => {
                    depth += 1;
                    saw_open = true;
                    pos += 1;
                }
                ')' | ']' | '}' => {
                    depth = depth.saturating_sub(1);
                    pos += 1;
                }
                _ => pos += 1,
            }
        }
        let continued =
            line.trim_end().ends_with('\\') || quote.is_some() || triple_quote.is_some();
        if idx == start_idx && !saw_open && !continued {
            return idx + 1;
        }
        if depth == 0 && !continued {
            return idx + 1;
        }
    }
    lines.len()
}

fn collect_classes(lines: &[&str], docs: &[Docstring]) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some((name, bases)) = parse_class_header(trimmed) else {
            continue;
        };
        let indent = indent_width(line);
        let decorators = decorators_before(lines, idx);
        let has_dataclass = decorators.iter().any(|decorator| {
            decorator.contains("dataclass") || decorator.contains("dataclass_transform")
        });
        let doc = docs
            .iter()
            .find(|doc| doc.owner.kind == OwnerKind::Class && doc.owner.line == idx + 1);
        let fields = class_fields(lines, idx + 1, indent);
        classes.push(ClassInfo {
            name,
            line: idx + 1,
            bases,
            has_dataclass,
            doc_start: doc.map(|doc| doc.start_line),
            doc_end: doc.map(|doc| doc.end_line),
            fields,
        });
    }
    classes
}

fn decorators_before(lines: &[&str], class_idx: usize) -> Vec<String> {
    let mut decorators = Vec::new();
    let mut idx = class_idx;
    while idx > 0 {
        idx -= 1;
        let trimmed = lines[idx].trim();
        if trimmed.starts_with('@') {
            decorators.push(trimmed.to_string());
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        break;
    }
    decorators
}

fn class_fields(lines: &[&str], class_line: usize, class_indent: usize) -> Vec<FieldInfo> {
    let mut fields = Vec::new();
    for line in lines.iter().skip(class_line) {
        if line.trim().is_empty()
            || line.trim_start().starts_with('#')
            || line.trim_start().starts_with('@')
        {
            continue;
        }
        let indent = indent_width(line);
        if indent <= class_indent {
            break;
        }
        if indent != class_indent + 4 {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("def ")
            || trimmed.starts_with("async def ")
            || trimmed.starts_with("class ")
        {
            continue;
        }
        let Some((name_part, rest)) = trimmed.split_once(':') else {
            continue;
        };
        let name = name_part.trim();
        if !is_identifier(name) {
            continue;
        }
        let ty = rest.split_once('=').map(|(left, _)| left).unwrap_or(rest);
        let ty = strip_inline_comment(ty).trim();
        if ty.is_empty() || ty.starts_with("ClassVar") {
            continue;
        }
        fields.push(FieldInfo {
            name: name.to_string(),
            ty: ty.to_string(),
        });
    }
    fields
}

fn inherited_fields(class: &ClassInfo, by_name: &HashMap<String, &ClassInfo>) -> Vec<FieldInfo> {
    fn visit(
        class: &ClassInfo,
        by_name: &HashMap<String, &ClassInfo>,
        seen: &mut HashSet<String>,
        out: &mut Vec<FieldInfo>,
    ) {
        if !seen.insert(class.name.clone()) {
            return;
        }
        for base in &class.bases {
            if let Some(base_class) = by_name.get(base) {
                visit(base_class, by_name, seen, out);
            }
        }
        for field in &class.fields {
            if let Some(pos) = out.iter().position(|old| old.name == field.name) {
                out[pos] = field.clone();
            } else {
                out.push(field.clone());
            }
        }
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    visit(class, by_name, &mut seen, &mut out);
    out
}

fn attribute_docs(doc: &Docstring) -> Vec<AttributeDoc> {
    let sections = sections(doc);
    let Some(section) = sections.iter().find(|section| section.name == "Attributes") else {
        return Vec::new();
    };
    let mut attrs = Vec::new();
    for idx in section.start_idx + 1..=section.end_idx {
        let line = &doc.content[idx];
        let trimmed = line.text.trim();
        if trimmed.is_empty() || section_heading_name(trimmed).is_some() {
            continue;
        }
        let Some((head, _desc)) = trimmed.split_once(':') else {
            continue;
        };
        let head = head.trim();
        if let Some(open) = head.find('(') {
            if let Some(close) = head.rfind(')') {
                attrs.push(AttributeDoc {
                    name: head[..open].trim().to_string(),
                    ty: Some(head[open + 1..close].trim().to_string()),
                    line: line.line,
                });
                continue;
            }
        }
        attrs.push(AttributeDoc {
            name: head.to_string(),
            ty: None,
            line: line.line,
        });
    }
    attrs
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

fn parse_class_header(trimmed: &str) -> Option<(String, Vec<String>)> {
    let text = trimmed.strip_prefix("class ")?;
    let end = text.find(['(', ':']).unwrap_or(text.len());
    let name = text[..end].trim();
    if !is_identifier(name) {
        return None;
    }
    let mut bases = Vec::new();
    if let Some(open) = text.find('(') {
        if let Some(close) = text[open + 1..].find(')') {
            let base_text = &text[open + 1..open + 1 + close];
            for base in base_text.split(',') {
                let base = base.trim();
                if base.is_empty() || base.contains('=') {
                    continue;
                }
                let short = base.rsplit('.').next().unwrap_or(base).trim();
                if is_identifier(short) {
                    bases.push(short.to_string());
                }
            }
        }
    }
    Some((name.to_string(), bases))
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic()) && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
}

fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum()
}

fn doc_line_col(line: &DocLine, relative_col: usize) -> usize {
    line.start_col + relative_col.saturating_sub(1)
}

fn doc_line_first_non_ws_col(line: &DocLine) -> usize {
    doc_line_col(line, first_non_ws_col(&line.text))
}

fn doc_line_end_col(line: &DocLine) -> usize {
    doc_line_col(line, line.text.chars().count() + 1)
}

fn doc_line_trimmed_end_col(line: &DocLine) -> usize {
    doc_line_col(line, line.text.trim_end().chars().count() + 1)
}

fn doc_line_trimmed_end_col_for(line_no: usize, text: &str, doc: &Docstring) -> usize {
    doc.content
        .iter()
        .find(|line| line.line == line_no)
        .map(doc_line_trimmed_end_col)
        .unwrap_or_else(|| text.trim_end().chars().count() + 1)
}

fn first_non_ws_col(text: &str) -> usize {
    text.chars().take_while(|ch| ch.is_whitespace()).count() + 1
}

fn strip_inline_comment(text: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (idx, ch) in text.char_indices() {
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
            '#' => return &text[..idx],
            '\'' | '"' => quote = Some(ch),
            _ => {}
        }
    }
    text
}

fn normalize_type(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .replace("typing.", "")
        .replace('"', "'")
}

fn doc_diag(
    code: &str,
    message: impl Into<String>,
    display_path: &str,
    span: Span,
    doc: &Docstring,
) -> Diagnostic {
    Diagnostic::new(code, message, display_path, span, "warning")
        .with_suppression_line(doc.end_line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EffectiveConfig, FileInlineConfig, PyProjectConfig, VscodeConfig};

    fn config() -> EffectiveConfig {
        EffectiveConfig::resolve(
            &VscodeConfig::default(),
            &PyProjectConfig::default(),
            &FileInlineConfig::default(),
        )
    }

    fn codes(source: &str) -> Vec<String> {
        run_docstring_rules(Path::new("example.py"), source, &config())
            .into_iter()
            .map(|diag| diag.code)
            .collect()
    }

    #[test]
    fn catches_missing_final_docstring() {
        assert!(codes("from typing import Final\nPORT: Final[int] = 1\n")
            .contains(&"SK609".to_string()));
    }

    #[test]
    fn catches_long_docstring_line() {
        let source = "def f():\n    \"\"\"Ожидание очень длинного описания которое точно выходит за лимит семьдесят два символа\"\"\"\n    pass\n";
        assert!(codes(source).contains(&"SK601".to_string()));
    }

    #[test]
    fn catches_lowercase_cyrillic_docstring_sentence() {
        let source = "def f():\n    \"\"\"ожидание обновления\"\"\"\n    pass\n";
        assert!(codes(source).contains(&"SK617".to_string()));
    }

    #[test]
    fn allows_continuation_line_inside_docstring_item() {
        let source = "def f():\n    \"\"\"\n    Описание\n\n    Args:\n        imued_frame_pos (IntVector2):\n            Позиция расположения OSD метаинформации кадра с IMU\n            на экране\n    \"\"\"\n    pass\n";
        assert!(!codes(source).contains(&"SK617".to_string()));
    }

    #[test]
    fn sk605_applies_only_to_functions_and_methods() {
        let class_doc = "class Box:\n    \"\"\"Ожидать обновления\"\"\"\n";
        assert!(!codes(class_doc).contains(&"SK605".to_string()));

        let const_doc =
            "from typing import Final\nPORT: Final[int] = 1\n\"\"\"Ожидать обновления\"\"\"\n";
        assert!(!codes(const_doc).contains(&"SK605".to_string()));

        let func_doc = "def f():\n    \"\"\"Ожидать обновления\"\"\"\n    pass\n";
        assert!(codes(func_doc).contains(&"SK605".to_string()));
    }

    #[test]
    fn sk606_checks_only_top_level_sections() {
        let source = r#"def f():
    """
    Описание

    Raises:
        ValueError:
            Если значение некорректно
    """
    pass
"#;
        assert!(!codes(source).contains(&"SK606".to_string()));
    }

    #[test]
    fn sk603_and_sk617_ignore_decimal_points() {
        let source = r#"def f():
    """
    Поддержка Python 3.14
    и новее
    """
    pass
"#;
        let found = codes(source);
        assert!(!found.contains(&"SK603".to_string()));
        assert!(!found.contains(&"SK617".to_string()));
    }

    #[test]
    fn sk603_and_sk617_ignore_known_abbreviations() {
        let source = r#"def f():
    """
    Поддержка Python 3.14 и т.д.
    работает дальше
    """
    pass
"#;
        let found = codes(source);
        assert!(!found.contains(&"SK603".to_string()));
        assert!(!found.contains(&"SK617".to_string()));
    }

    #[test]
    fn sk609_accepts_docstring_after_multiline_final_string_assignment() {
        let source = r#"from typing import Final

_COPY_METHOD_SOURCE: Final[str] = """
def copy(self):
    return self
"""
"""Шаблон исходного кода"""
"#;
        assert!(!codes(source).contains(&"SK609".to_string()));
    }

    #[test]
    fn sk620_ignores_inline_field_comments_and_quote_style() {
        let source = r#"from dataclasses import dataclass
from typing import Literal

@dataclass
class Config:
    """
    Конфигурация

    Attributes:
        parity (Literal['N', 'E']): Чётность
    """

    parity: Literal["N", "E"]  # Чётность
"#;
        assert!(!codes(source).contains(&"SK620".to_string()));
    }

    #[test]
    fn sk609_accepts_docstring_after_multiline_final_assignment() {
        let source = r#"from typing import Final

_LAZY_IMPORTS: Final[LazyImport] = LazyImport({
    "RcChannels": "sk.collections",
    "TargetId": "sk.collections",
})
"""Ленивые импорты типов, создающих цикличный импорт"""
"#;
        assert!(!codes(source).contains(&"SK609".to_string()));
    }

    #[test]
    fn sk609_still_requires_docstring_after_multiline_final_assignment() {
        let source = r#"from typing import Final

_LAZY_IMPORTS: Final[LazyImport] = LazyImport({
    "RcChannels": "sk.collections",
    "TargetId": "sk.collections",
})
print(_LAZY_IMPORTS)
"#;
        assert!(codes(source).contains(&"SK609".to_string()));
    }

    #[test]
    fn sk615_ignores_nested_function_and_class_docstrings() {
        let source = r#"def outer():
    def inner():
        """
        Описание
        
        Args:
            value (int): Значение
        """
        return 1

    class Nested:
        """
        Описание
        
        Attributes:
            value (int): Значение
        """

        value: int

    return inner(), Nested
"#;
        assert!(!codes(source).contains(&"SK615".to_string()));
    }

    #[test]
    fn sk613_ignores_docstring_that_is_the_last_class_statement() {
        let source = r#"if TYPE_CHECKING:
    class _InitializableHandlerMeta(DataDictMeta, InitializableMeta):
        """
        Общий метакласс для инициализируемых классов
        для корректной проверки типов
        """
    class _ABCHandlerMeta(DataDictMeta, ABCMeta):
        """
        Общий метакласс для инициализируемых классов
        для корректной проверки типов
        """
"#;
        assert!(!codes(source).contains(&"SK613".to_string()));
    }

    #[test]
    fn sk613_ignores_one_line_docstring_before_body_statement() {
        let source = r#"def f():
    """Описание"""
    return 1
"#;
        assert!(!codes(source).contains(&"SK613".to_string()));
    }

    #[test]
    fn sk613_still_requires_blank_line_before_body_statement() {
        let source = r#"def f():
    """
    Описание
    """
    return 1
"#;
        assert!(codes(source).contains(&"SK613".to_string()));
    }

    #[test]
    fn catches_dataclass_inherited_attribute() {
        let source = "from dataclasses import dataclass\n\n@dataclass\nclass Base:\n    \"\"\"\n    Описание\n\n    Attributes:\n        x (int): значение\n    \"\"\"\n    x: int\n\n@dataclass\nclass Child(Base):\n    \"\"\"\n    Описание\n\n    Attributes:\n        y (str): значение\n    \"\"\"\n    y: str\n";
        assert!(codes(source).contains(&"SK619".to_string()));
    }

    #[test]
    fn sk607_ignores_quote_boundary_lines_of_multiline_nested_docstring() {
        let source = r#"def outer():
    class Common:
        """
        Общие параметры конфигурации модулей.
        Переопределяют локальные параметры модулей
        """

    return Common
"#;
        assert!(!codes(source).contains(&"SK607".to_string()));
    }

    #[test]
    fn sk607_still_catches_real_blank_lines_inside_nested_docstring() {
        let source = r#"def outer():
    class Common:
        """
        Общие параметры конфигурации модулей

        Переопределяют локальные параметры модулей
        """

    return Common
"#;
        assert!(codes(source).contains(&"SK607".to_string()));
    }

    #[test]
    fn sk613_ignores_stub_function_docstring_before_ellipsis() {
        let source = r#"def f():
    """Описание"""
    ..."#;
        assert!(!codes(source).contains(&"SK613".to_string()));
    }

    #[test]
    fn sk609_ignores_docstring_text_and_all_tuple() {
        let source = "from typing import Final\n\n__all__: Final[tuple[str, ...]] = (\n    \"Public\"\n)\n\ndef f():\n    \"\"\"\n    Returns:\n        Value: Final[Описание]\n    \"\"\"\n    pass\n";
        assert!(!codes(source).contains(&"SK609".to_string()));
    }
}
