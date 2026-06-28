use crate::config::EffectiveConfig;
use crate::diagnostic::{Diagnostic, Fix, Span};
use std::path::Path;

pub fn run_comment_rules(path: &Path, source: &str, config: &EffectiveConfig) -> Vec<Diagnostic> {
    let display_path = path.display().to_string();
    let lines: Vec<&str> = source.lines().collect();
    let ignored = triple_string_lines(&lines);
    let mut diagnostics = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        if ignored.get(idx).copied().unwrap_or(false) {
            continue;
        }
        let Some(hash_byte) = comment_hash_byte(line) else {
            continue;
        };
        let line_no = idx + 1;
        let comment = &line[hash_byte + 1..];
        if is_directive_comment(line_no, line, comment) {
            continue;
        }

        if config.is_enabled("SK211") {
            run_comment_capitalization(
                &display_path,
                line_no,
                line,
                hash_byte,
                comment,
                &mut diagnostics,
            );
        }
        if config.is_enabled("SK212") {
            run_comment_trailing_period(
                &display_path,
                line_no,
                line,
                hash_byte,
                comment,
                &mut diagnostics,
            );
        }
    }

    diagnostics
}

fn run_comment_capitalization(
    display_path: &str,
    line_no: usize,
    line: &str,
    hash_byte: usize,
    comment: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut sentence_start = true;
    for (rel_byte, ch) in comment.char_indices() {
        if ch.is_whitespace() || ch == '#' {
            continue;
        }
        if sentence_start && is_cyrillic_lower(ch) {
            let column = line[..hash_byte + 1 + rel_byte].chars().count() + 1;
            diagnostics.push(
                Diagnostic::new(
                    "SK211",
                    "Cyrillic comment sentences must start with an uppercase letter",
                    display_path,
                    Span::new(line_no, column, line_no, column + 1),
                    "warning",
                )
                .with_fix(Fix {
                    message: "Uppercase the first Cyrillic letter".to_string(),
                    replacement: ch.to_uppercase().collect::<String>(),
                    start_line: line_no,
                    start_column: column,
                    end_line: line_no,
                    end_column: column + 1,
                }),
            );
            break;
        }
        sentence_start =
            matches!(ch, '!' | '?') || (ch == '.' && period_is_sentence_ending(comment, rel_byte));
    }
}

fn run_comment_trailing_period(
    display_path: &str,
    line_no: usize,
    line: &str,
    hash_byte: usize,
    comment: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let trimmed = comment.trim_end_matches([' ', '\t']);
    if trimmed.ends_with('.')
        && !trimmed.ends_with("...")
        && final_period_is_sentence_ending(trimmed)
    {
        let rel_len = trimmed.chars().count();
        let column = line[..hash_byte + 1].chars().count() + rel_len;
        diagnostics.push(
            Diagnostic::new(
                "SK212",
                "Comments must not end with a period",
                display_path,
                Span::new(line_no, column, line_no, column + 1),
                "warning",
            )
            .with_fix(Fix {
                message: "Remove the final period from the comment".to_string(),
                replacement: String::new(),
                start_line: line_no,
                start_column: column,
                end_line: line_no,
                end_column: column + 1,
            }),
        );
    }
}

fn is_directive_comment(line_no: usize, line: &str, comment: &str) -> bool {
    let trimmed = comment.trim_start();
    if line_no == 1 && line.starts_with("#!") {
        return true;
    }
    if line_no <= 2 && is_encoding_comment(trimmed) {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("noqa")
        || lower.starts_with("sklint:")
        || lower.starts_with("type: ignore")
        || lower.starts_with("pyright:")
        || lower.starts_with("pylance:")
        || lower.starts_with("pylint:")
        || lower.starts_with("ruff:")
        || lower.starts_with("fmt:")
        || lower.starts_with("isort:")
        || lower.starts_with("mypy:")
}

fn is_encoding_comment(comment: &str) -> bool {
    let lower = comment.to_ascii_lowercase();
    lower.contains("coding:") || lower.contains("coding=")
}

fn comment_hash_byte(line: &str) -> Option<usize> {
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
            '#' => return Some(idx),
            '\'' | '"' => {
                if line[idx..].starts_with("'''") || line[idx..].starts_with("\"\"\"") {
                    return None;
                }
                quote = Some(ch);
            }
            _ => {}
        }
    }
    None
}

fn triple_string_lines(lines: &[&str]) -> Vec<bool> {
    let mut ignored = vec![false; lines.len()];
    let mut in_triple: Option<&str> = None;
    for (idx, line) in lines.iter().enumerate() {
        if let Some(quote) = in_triple {
            ignored[idx] = true;
            if line.contains(quote) {
                in_triple = None;
            }
            continue;
        }
        let double = line.find("\"\"\"");
        let single = line.find("'''");
        let Some((pos, quote)) = earliest_quote(double, single) else {
            continue;
        };
        let after = &line[pos + 3..];
        ignored[idx] = true;
        if !after.contains(quote) {
            in_triple = Some(quote);
        }
    }
    ignored
}

fn earliest_quote(double: Option<usize>, single: Option<usize>) -> Option<(usize, &'static str)> {
    match (double, single) {
        (Some(d), Some(s)) if d < s => Some((d, "\"\"\"")),
        (Some(_), Some(s)) => Some((s, "'''")),
        (Some(d), None) => Some((d, "\"\"\"")),
        (None, Some(s)) => Some((s, "'''")),
        (None, None) => None,
    }
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

fn is_cyrillic(ch: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&ch) || ('\u{0500}'..='\u{052F}').contains(&ch)
}

fn is_cyrillic_lower(ch: char) -> bool {
    is_cyrillic(ch) && ch.is_lowercase()
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
        run_comment_rules(Path::new("example.py"), source, &config())
            .into_iter()
            .map(|diag| diag.code)
            .collect()
    }

    #[test]
    fn catches_lowercase_cyrillic_comment_sentence() {
        assert!(codes("# комментарий\n").contains(&"SK211".to_string()));
    }

    #[test]
    fn catches_comment_trailing_period() {
        assert!(codes("x = 1  # Комментарий.\n").contains(&"SK212".to_string()));
    }

    #[test]
    fn allows_comment_period_after_digit() {
        assert!(!codes("x = 1  # Версия Python 3.14\n").contains(&"SK212".to_string()));
    }

    #[test]
    fn period_after_digit_does_not_start_comment_sentence() {
        assert!(!codes("x = 1  # Версия Python 3.14 работает\n").contains(&"SK211".to_string()));
    }

    #[test]
    fn allows_known_abbreviation_comment_periods() {
        let found = codes("x = 1  # Поддерживает Python 3.14 и т.д.\n");
        assert!(!found.contains(&"SK211".to_string()));
        assert!(!found.contains(&"SK212".to_string()));
    }

    #[test]
    fn known_abbreviation_period_does_not_start_comment_sentence() {
        assert!(
            !codes("x = 1  # Поддерживает Python 3.14 и т.д. работает\n")
                .contains(&"SK211".to_string())
        );
    }

    #[test]
    fn ignores_noqa_directive_comment() {
        assert!(codes("x = 1  # noqa: SK001.\n").is_empty());
    }
}
