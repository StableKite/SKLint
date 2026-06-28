use crate::config::{parse_csv_codes, sklint_directive};
use crate::rules::code_matches_selector;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionKind {
    LineNoqa,
    LineSklint,
    File,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppression {
    pub id: usize,
    pub kind: SuppressionKind,
    pub line: usize,
    pub codes: Vec<String>,
    pub text: String,
    pub hits: usize,
}

impl Suppression {
    pub fn matches(&self, code: &str) -> bool {
        self.codes.is_empty()
            || self
                .codes
                .iter()
                .any(|selector| code_matches_selector(code, selector))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuppressionState {
    pub suppressions: Vec<Suppression>,
    block_events: Vec<BlockEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockEvent {
    suppression_id: Option<usize>,
    is_enable: bool,
    line: usize,
    codes: Vec<String>,
}

impl SuppressionState {
    pub fn parse(source: &str) -> Self {
        let mut suppressions = Vec::new();
        let mut block_events = Vec::new();
        let mut first_code_line_seen = false;
        let docstring_last_content_to_closing = docstring_last_content_to_closing_lines(source);

        for (idx, line) in source.lines().enumerate() {
            let line_no = idx + 1;
            let Some(comment_start) = line.find('#') else {
                if !line.trim().is_empty() {
                    first_code_line_seen = true;
                }
                continue;
            };

            let comment = &line[comment_start..];
            let line_suppression_line = docstring_last_content_to_closing
                .get(&line_no)
                .copied()
                .unwrap_or(line_no);

            if !first_code_line_seen && line[..comment_start].trim().is_empty() {
                for segment in hash_comment_segments(comment) {
                    if let Some(codes) = parse_file_noqa(segment) {
                        let id = suppressions.len();
                        suppressions.push(Suppression {
                            id,
                            kind: SuppressionKind::File,
                            line: line_no,
                            codes,
                            text: segment.trim_start().to_string(),
                            hits: 0,
                        });
                    }
                }
            }

            for segment in hash_comment_segments(comment) {
                let trimmed_segment = segment.trim_start();
                if let Some(codes) = parse_noqa(trimmed_segment) {
                    if !codes.is_empty() {
                        let id = suppressions.len();
                        suppressions.push(Suppression {
                            id,
                            kind: SuppressionKind::LineNoqa,
                            line: line_suppression_line,
                            codes,
                            text: trimmed_segment.to_string(),
                            hits: 0,
                        });
                    }
                }

                let Some(directive) = sklint_directive(trimmed_segment) else {
                    continue;
                };
                let lower = directive.to_ascii_lowercase();
                if lower.starts_with("ignore") && !lower.starts_with("ignore=") {
                    let codes = directive
                        .strip_prefix("ignore")
                        .or_else(|| directive.strip_prefix("IGNORE"))
                        .unwrap_or("")
                        .trim_start_matches(|ch: char| {
                            ch == ':' || ch == '=' || ch.is_ascii_whitespace()
                        });
                    let id = suppressions.len();
                    suppressions.push(Suppression {
                        id,
                        kind: SuppressionKind::LineSklint,
                        line: line_suppression_line,
                        codes: parse_csv_codes(codes),
                        text: trimmed_segment.to_string(),
                        hits: 0,
                    });
                } else if lower.starts_with("disable") {
                    let codes_text = directive
                        .strip_prefix("disable")
                        .or_else(|| directive.strip_prefix("DISABLE"))
                        .unwrap_or("")
                        .trim_start_matches(|ch: char| {
                            ch == ':' || ch == '=' || ch.is_ascii_whitespace()
                        });
                    let codes = parse_csv_codes(codes_text);
                    let id = suppressions.len();
                    suppressions.push(Suppression {
                        id,
                        kind: SuppressionKind::Block,
                        line: line_no,
                        codes: codes.clone(),
                        text: trimmed_segment.to_string(),
                        hits: 0,
                    });
                    block_events.push(BlockEvent {
                        suppression_id: Some(id),
                        is_enable: false,
                        line: line_no,
                        codes,
                    });
                } else if lower.starts_with("enable") {
                    let codes_text = directive
                        .strip_prefix("enable")
                        .or_else(|| directive.strip_prefix("ENABLE"))
                        .unwrap_or("")
                        .trim_start_matches(|ch: char| {
                            ch == ':' || ch == '=' || ch.is_ascii_whitespace()
                        });
                    block_events.push(BlockEvent {
                        suppression_id: None,
                        is_enable: true,
                        line: line_no,
                        codes: parse_csv_codes(codes_text),
                    });
                }
            }

            if !line.trim().is_empty() && !line.trim_start().starts_with('#') {
                first_code_line_seen = true;
            }
        }

        Self {
            suppressions,
            block_events,
        }
    }

    pub fn suppressing_ids_for(
        &self,
        line: usize,
        code: &str,
        exclude_id: Option<usize>,
    ) -> Vec<usize> {
        let mut ids = Vec::new();

        for suppression in &self.suppressions {
            if Some(suppression.id) == exclude_id || !suppression.matches(code) {
                continue;
            }

            match suppression.kind {
                SuppressionKind::File => ids.push(suppression.id),
                SuppressionKind::LineNoqa | SuppressionKind::LineSklint => {
                    if suppression.line == line {
                        ids.push(suppression.id);
                    }
                }
                SuppressionKind::Block => {}
            }
        }

        ids.extend(self.active_block_suppressions_for(line, code, exclude_id));
        ids.sort();
        ids.dedup();
        ids
    }

    pub fn mark_hits(&mut self, ids: &[usize]) {
        let mut by_id: HashMap<usize, usize> = HashMap::new();
        for id in ids {
            *by_id.entry(*id).or_default() += 1;
        }
        for suppression in &mut self.suppressions {
            if let Some(hits) = by_id.get(&suppression.id) {
                suppression.hits += hits;
            }
        }
    }

    fn active_block_suppressions_for(
        &self,
        line: usize,
        code: &str,
        exclude_id: Option<usize>,
    ) -> Vec<usize> {
        let mut active: Vec<(usize, Vec<String>)> = Vec::new();

        for event in self.block_events.iter().filter(|event| event.line <= line) {
            if event.is_enable {
                let enable_codes = &event.codes;
                active.retain(|(_, codes)| {
                    if enable_codes.is_empty() {
                        return false;
                    }
                    !selector_lists_intersect(codes, enable_codes)
                });
            } else if let Some(id) = event.suppression_id {
                if Some(id) != exclude_id {
                    active.push((id, event.codes.clone()));
                }
            }
        }

        active
            .into_iter()
            .filter(|(_, codes)| {
                codes.is_empty()
                    || codes
                        .iter()
                        .any(|selector| code_matches_selector(code, selector))
            })
            .map(|(id, _)| id)
            .collect()
    }
}

fn hash_comment_segments(comment: &str) -> Vec<&str> {
    comment
        .match_indices('#')
        .map(|(idx, _)| &comment[idx..])
        .collect()
}

fn docstring_last_content_to_closing_lines(source: &str) -> HashMap<usize, usize> {
    let lines: Vec<&str> = source.lines().collect();
    let mut mapping = HashMap::new();
    let mut scan = 0usize;

    while scan < lines.len() {
        let trimmed = lines[scan].trim_start();
        let Some(quote) = triple_quote_prefix(trimmed) else {
            scan += 1;
            continue;
        };

        if trimmed[quote.len()..].contains(quote) {
            scan += 1;
            continue;
        }

        let Some(end) = ((scan + 1)..lines.len()).find(|idx| lines[*idx].contains(quote)) else {
            break;
        };

        if let Some(last_content) = ((scan + 1)..end)
            .rev()
            .find(|idx| !lines[*idx].trim().is_empty())
        {
            mapping.insert(last_content + 1, end + 1);
        }

        scan = end + 1;
    }

    mapping
}

fn triple_quote_prefix(trimmed: &str) -> Option<&'static str> {
    if trimmed.starts_with("\"\"\"") {
        Some("\"\"\"")
    } else if trimmed.starts_with("'''") {
        Some("'''")
    } else {
        None
    }
}

fn parse_file_noqa(comment: &str) -> Option<Vec<String>> {
    let directive = sklint_directive(comment)?;
    let lower = directive.to_ascii_lowercase();
    if !lower.starts_with("noqa") {
        return None;
    }
    let codes = directive
        .split_once(':')
        .map(|(_, rest)| parse_csv_codes(rest))
        .unwrap_or_default();
    Some(codes)
}

fn parse_noqa(comment: &str) -> Option<Vec<String>> {
    let text = comment.trim_start().strip_prefix('#')?.trim_start();
    let lower = text.to_ascii_lowercase();
    if !lower.starts_with("noqa") {
        return None;
    }
    if let Some((_, rest)) = text.split_once(':') {
        let codes = parse_csv_codes(rest)
            .into_iter()
            .filter(|code| code.starts_with("SK"))
            .collect();
        Some(codes)
    } else {
        // Deliberately do not treat bare `# noqa` as an SKLint suppression:
        // SKLint only consumes explicit SKxxx selectors so it can coexist with
        // flake8/ruff directives without changing their meaning.
        Some(Vec::new())
    }
}

fn selector_lists_intersect(left: &[String], right: &[String]) -> bool {
    if left.is_empty() || right.is_empty() {
        return true;
    }

    left.iter().any(|a| {
        right
            .iter()
            .any(|b| a == b || a.starts_with(b) || b.starts_with(a))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noqa_consumes_only_sk_codes() {
        let state = SuppressionState::parse("x = 1  # noqa: E501, SK001\n");
        assert_eq!(state.suppressions.len(), 1);
        assert_eq!(state.suppressions[0].codes, vec!["SK001"]);
    }

    #[test]
    fn bare_noqa_is_ignored() {
        let state = SuppressionState::parse("x = 1  # noqa\n");
        assert!(state.suppressions.is_empty());
    }

    #[test]
    fn block_disable_then_enable() {
        let state = SuppressionState::parse(
            "# sklint: disable=SK001\nx = 1\n# sklint: enable=SK001\ny = 2\n",
        );
        assert_eq!(state.suppressing_ids_for(2, "SK001", None).len(), 1);
        assert!(state.suppressing_ids_for(4, "SK001", None).is_empty());
    }

    #[test]
    fn noqa_after_existing_comment_is_parsed() {
        let state = SuppressionState::parse("x=1  # pyright: ignore[reportAny]  # noqa: SK401\n");
        assert_eq!(state.suppressions.len(), 1);
        assert_eq!(state.suppressing_ids_for(1, "SK401", None).len(), 1);
    }

    #[test]
    fn sklint_ignore_after_existing_comment_is_parsed() {
        let state =
            SuppressionState::parse("x=1  # pyright: ignore[reportAny]  # sklint: ignore SK401\n");
        assert_eq!(state.suppressions.len(), 1);
        assert_eq!(state.suppressing_ids_for(1, "SK401", None).len(), 1);
    }

    #[test]
    fn docstring_last_content_line_suppresses_closing_line() {
        let state = SuppressionState::parse(
            "def f():\n    \"\"\"\n    описание.  # noqa: SK617\n    \"\"\"\n    pass\n",
        );
        assert_eq!(state.suppressions.len(), 1);
        assert_eq!(state.suppressing_ids_for(4, "SK617", None).len(), 1);
    }
}
