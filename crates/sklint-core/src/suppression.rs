use crate::config::{parse_csv_codes, sklint_directive};
use crate::python_ast::PythonAst;
use crate::rules::code_matches_selector;
use rustpython_parser::ast::{self, Stmt};

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
    pub selector_hits: Vec<usize>,
    pub catch_all_hits: usize,
    /// Optional multiline simple-statement scope for explicit SK901 local
    /// suppressions placed on the statement's first or closing line.
    pub statement_scope: Option<(usize, usize)>,
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
        let comment_starts = python_comment_starts(source);

        for (idx, line) in source.lines().enumerate() {
            let line_no = idx + 1;
            let Some(comment_start) = comment_starts.get(idx).copied().flatten() else {
                if !line.trim().is_empty() {
                    first_code_line_seen = true;
                }
                continue;
            };

            let comment = &line[comment_start..];
            let line_suppression_line = line_no;

            if !first_code_line_seen && line[..comment_start].trim().is_empty() {
                for segment in hash_comment_segments(comment) {
                    if let Some(codes) = parse_file_noqa(segment) {
                        let id = suppressions.len();
                        let selector_hits = vec![0; codes.len()];
                        suppressions.push(Suppression {
                            id,
                            kind: SuppressionKind::File,
                            line: line_no,
                            codes,
                            text: segment.trim_start().to_string(),
                            hits: 0,
                            selector_hits,
                            catch_all_hits: 0,
                            statement_scope: None,
                        });
                    }
                }
            }

            for segment in hash_comment_segments(comment) {
                let trimmed_segment = segment.trim_start();
                if let Some(codes) = parse_noqa(trimmed_segment) {
                    if !codes.is_empty() {
                        let id = suppressions.len();
                        let selector_hits = vec![0; codes.len()];
                        suppressions.push(Suppression {
                            id,
                            kind: SuppressionKind::LineNoqa,
                            line: line_suppression_line,
                            codes,
                            text: trimmed_segment.to_string(),
                            hits: 0,
                            selector_hits,
                            catch_all_hits: 0,
                            statement_scope: None,
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
                    let codes = parse_csv_codes(codes);
                    let selector_hits = vec![0; codes.len()];
                    let id = suppressions.len();
                    suppressions.push(Suppression {
                        id,
                        kind: SuppressionKind::LineSklint,
                        line: line_suppression_line,
                        codes,
                        text: trimmed_segment.to_string(),
                        hits: 0,
                        selector_hits,
                        catch_all_hits: 0,
                        statement_scope: None,
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
                        selector_hits: vec![0; codes.len()],
                        catch_all_hits: 0,
                        statement_scope: None,
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

        attach_statement_scopes(source, &mut suppressions);

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
        let mut ids = self
            .suppressing_matches_for(line, code, exclude_id)
            .into_iter()
            .map(|candidate| candidate.suppression_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    pub fn suppressing_matches_for(
        &self,
        line: usize,
        code: &str,
        exclude_id: Option<usize>,
    ) -> Vec<SuppressionMatch> {
        let mut matches = Vec::new();

        for suppression in &self.suppressions {
            if Some(suppression.id) == exclude_id || suppression.kind == SuppressionKind::Block {
                continue;
            }
            let applies_here = match suppression.kind {
                SuppressionKind::File => true,
                SuppressionKind::LineNoqa | SuppressionKind::LineSklint => {
                    suppression.line == line
                        || (code == "SK901"
                            && suppression
                                .statement_scope
                                .is_some_and(|(start, end)| start <= line && line <= end))
                }
                SuppressionKind::Block => false,
            };
            if applies_here {
                matches.extend(matches_for_suppression(suppression, code));
            }
        }

        matches.extend(self.active_block_matches_for(line, code, exclude_id));
        matches
    }

    pub fn responsible_match_for(
        &self,
        line: usize,
        code: &str,
        exclude_id: Option<usize>,
    ) -> Option<SuppressionMatch> {
        self.suppressing_matches_for(line, code, exclude_id)
            .into_iter()
            .max_by_key(|candidate| self.responsibility_score(candidate))
    }

    pub fn mark_match(&mut self, candidate: SuppressionMatch) {
        let Some(suppression) = self
            .suppressions
            .iter_mut()
            .find(|suppression| suppression.id == candidate.suppression_id)
        else {
            return;
        };
        suppression.hits += 1;
        if let Some(index) = candidate.selector_index {
            if let Some(hits) = suppression.selector_hits.get_mut(index) {
                *hits += 1;
            }
        } else {
            suppression.catch_all_hits += 1;
        }
    }

    pub fn selector_is_used(&self, suppression_id: usize, selector_index: Option<usize>) -> bool {
        let Some(suppression) = self
            .suppressions
            .iter()
            .find(|suppression| suppression.id == suppression_id)
        else {
            return false;
        };
        match selector_index {
            Some(index) => suppression.selector_hits.get(index).copied().unwrap_or(0) > 0,
            None => suppression.catch_all_hits > 0,
        }
    }

    fn responsibility_score(&self, candidate: &SuppressionMatch) -> (u8, usize, usize, usize) {
        let suppression = &self.suppressions[candidate.suppression_id];
        let locality = match suppression.kind {
            SuppressionKind::File => 0,
            SuppressionKind::Block => 1,
            SuppressionKind::LineNoqa | SuppressionKind::LineSklint => 2,
        };
        let specificity = candidate
            .selector_index
            .and_then(|index| suppression.codes.get(index))
            .map(|selector| selector_specificity(selector))
            .unwrap_or(0);
        (locality, specificity, suppression.line, suppression.id)
    }

    fn active_block_matches_for(
        &self,
        line: usize,
        code: &str,
        exclude_id: Option<usize>,
    ) -> Vec<SuppressionMatch> {
        let mut active = Vec::<SuppressionMatch>::new();

        for event in self.block_events.iter().filter(|event| event.line <= line) {
            if event.is_enable {
                if event.codes.is_empty()
                    || event
                        .codes
                        .iter()
                        .any(|selector| code_matches_selector(code, selector))
                {
                    // Evaluate block state for the requested diagnostic code. This
                    // preserves `disable` catch-all + `enable SK401`: SK401 is
                    // re-enabled while every other code remains disabled.
                    active.clear();
                }
                continue;
            }

            let Some(id) = event.suppression_id else {
                continue;
            };
            if Some(id) == exclude_id {
                continue;
            }
            let suppression = &self.suppressions[id];
            active.extend(matches_for_suppression(suppression, code));
        }

        active
    }
}

fn attach_statement_scopes(source: &str, suppressions: &mut [Suppression]) {
    let Ok(ast) = PythonAst::parse(source, "<suppression-scope>") else {
        return;
    };
    let mut ranges = Vec::new();
    collect_simple_statement_ranges(&ast.suite, &ast, &mut ranges);

    for suppression in suppressions {
        if !matches!(
            suppression.kind,
            SuppressionKind::LineNoqa | SuppressionKind::LineSklint
        ) || !suppression
            .codes
            .iter()
            .any(|selector| code_matches_selector("SK901", selector))
        {
            continue;
        }

        suppression.statement_scope = ranges
            .iter()
            .copied()
            .filter(|(start, end)| {
                *start < *end && (suppression.line == *start || suppression.line == *end)
            })
            .filter(|(start, end)| *start <= suppression.line && suppression.line <= *end)
            .min_by_key(|(start, end)| end - start);
    }
}

fn collect_simple_statement_ranges(
    statements: &[Stmt],
    ast: &PythonAst,
    ranges: &mut Vec<(usize, usize)>,
) {
    for statement in statements {
        if is_statement_scope_owner(statement) {
            let start = ast.location_of(statement).line;
            let end = ast.end_location_of(statement).line;
            if end > start {
                ranges.push((start, end));
            }
        }

        match statement {
            Stmt::FunctionDef(node) => collect_simple_statement_ranges(&node.body, ast, ranges),
            Stmt::AsyncFunctionDef(node) => {
                collect_simple_statement_ranges(&node.body, ast, ranges)
            }
            Stmt::ClassDef(node) => collect_simple_statement_ranges(&node.body, ast, ranges),
            Stmt::For(node) => {
                collect_simple_statement_ranges(&node.body, ast, ranges);
                collect_simple_statement_ranges(&node.orelse, ast, ranges);
            }
            Stmt::AsyncFor(node) => {
                collect_simple_statement_ranges(&node.body, ast, ranges);
                collect_simple_statement_ranges(&node.orelse, ast, ranges);
            }
            Stmt::While(node) => {
                collect_simple_statement_ranges(&node.body, ast, ranges);
                collect_simple_statement_ranges(&node.orelse, ast, ranges);
            }
            Stmt::If(node) => {
                collect_simple_statement_ranges(&node.body, ast, ranges);
                collect_simple_statement_ranges(&node.orelse, ast, ranges);
            }
            Stmt::With(node) => collect_simple_statement_ranges(&node.body, ast, ranges),
            Stmt::AsyncWith(node) => collect_simple_statement_ranges(&node.body, ast, ranges),
            Stmt::Match(node) => {
                for case in &node.cases {
                    collect_simple_statement_ranges(&case.body, ast, ranges);
                }
            }
            Stmt::Try(node) => {
                collect_simple_statement_ranges(&node.body, ast, ranges);
                for handler in &node.handlers {
                    let ast::ExceptHandler::ExceptHandler(handler) = handler;
                    collect_simple_statement_ranges(&handler.body, ast, ranges);
                }
                collect_simple_statement_ranges(&node.orelse, ast, ranges);
                collect_simple_statement_ranges(&node.finalbody, ast, ranges);
            }
            Stmt::TryStar(node) => {
                collect_simple_statement_ranges(&node.body, ast, ranges);
                for handler in &node.handlers {
                    let ast::ExceptHandler::ExceptHandler(handler) = handler;
                    collect_simple_statement_ranges(&handler.body, ast, ranges);
                }
                collect_simple_statement_ranges(&node.orelse, ast, ranges);
                collect_simple_statement_ranges(&node.finalbody, ast, ranges);
            }
            Stmt::Return(_)
            | Stmt::Delete(_)
            | Stmt::Assign(_)
            | Stmt::TypeAlias(_)
            | Stmt::AugAssign(_)
            | Stmt::AnnAssign(_)
            | Stmt::Raise(_)
            | Stmt::Assert(_)
            | Stmt::Import(_)
            | Stmt::ImportFrom(_)
            | Stmt::Global(_)
            | Stmt::Nonlocal(_)
            | Stmt::Expr(_)
            | Stmt::Pass(_)
            | Stmt::Break(_)
            | Stmt::Continue(_) => {}
        }
    }
}

fn is_statement_scope_owner(statement: &Stmt) -> bool {
    matches!(
        statement,
        Stmt::Return(_)
            | Stmt::Delete(_)
            | Stmt::Assign(_)
            | Stmt::TypeAlias(_)
            | Stmt::AugAssign(_)
            | Stmt::AnnAssign(_)
            | Stmt::Raise(_)
            | Stmt::Assert(_)
            | Stmt::Expr(_)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuppressionMatch {
    pub suppression_id: usize,
    pub selector_index: Option<usize>,
}

fn matches_for_suppression(suppression: &Suppression, code: &str) -> Vec<SuppressionMatch> {
    if suppression.codes.is_empty() {
        return vec![SuppressionMatch {
            suppression_id: suppression.id,
            selector_index: None,
        }];
    }
    suppression
        .codes
        .iter()
        .enumerate()
        .filter(|(_, selector)| code_matches_selector(code, selector))
        .map(|(index, _)| SuppressionMatch {
            suppression_id: suppression.id,
            selector_index: Some(index),
        })
        .collect()
}

fn selector_specificity(selector: &str) -> usize {
    let selector = selector.trim().to_ascii_uppercase();
    if selector == "ALL" {
        0
    } else if let Some(suffix) = selector.strip_prefix("DOC") {
        3 + suffix.len()
    } else {
        selector.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PythonStringState {
    quote: u8,
    triple: bool,
}

fn python_comment_starts(source: &str) -> Vec<Option<usize>> {
    let mut state: Option<PythonStringState> = None;
    source
        .lines()
        .map(|line| python_comment_start(line, &mut state))
        .collect()
}

fn python_comment_start(line: &str, state: &mut Option<PythonStringState>) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if let Some(string) = *state {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
                continue;
            }
            if bytes[index] == string.quote {
                if string.triple {
                    if index + 2 < bytes.len()
                        && bytes[index + 1] == string.quote
                        && bytes[index + 2] == string.quote
                    {
                        *state = None;
                        index += 3;
                        continue;
                    }
                } else {
                    *state = None;
                    index += 1;
                    continue;
                }
            }
            index += 1;
            continue;
        }

        match bytes[index] {
            b'#' => return Some(index),
            quote @ (b'\'' | b'"') => {
                let triple = index + 2 < bytes.len()
                    && bytes[index + 1] == quote
                    && bytes[index + 2] == quote;
                *state = Some(PythonStringState { quote, triple });
                index += if triple { 3 } else { 1 };
            }
            _ => index += 1,
        }
    }

    None
}

fn hash_comment_segments(comment: &str) -> Vec<&str> {
    let starts = comment
        .match_indices('#')
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    starts
        .iter()
        .enumerate()
        .map(|(position, start)| {
            let end = starts.get(position + 1).copied().unwrap_or(comment.len());
            &comment[*start..end]
        })
        .collect()
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
    let remainder = noqa_remainder(text)?;
    let Some(rest) = remainder.trim_start().strip_prefix(':') else {
        // Deliberately do not treat bare `# noqa` as an SKLint suppression:
        // SKLint consumes explicit SKxxx/SKDxxx selectors and DOCxxx aliases.
        // Foreign Ruff/Flake8 selectors keep their ownership.
        return Some(Vec::new());
    };
    let (selectors, _) = split_noqa_reason(rest);
    let upper = selectors.to_ascii_uppercase();
    let mut positioned = parse_csv_codes(selectors)
        .into_iter()
        .filter(|code| code.starts_with("SK"))
        .map(|code| (upper.find(&code).unwrap_or(usize::MAX), code))
        .collect::<Vec<_>>();
    positioned.extend(
        extract_native_doc_codes(selectors)
            .into_iter()
            .map(|code| (upper.find(&code).unwrap_or(usize::MAX), code)),
    );
    positioned.sort_by_key(|(position, _)| *position);

    let mut codes = Vec::new();
    for (_, code) in positioned {
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    Some(codes)
}

fn extract_native_doc_codes(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut codes = Vec::new();
    let mut index = 0usize;

    while index + 3 < bytes.len() {
        if bytes[index..index + 3].eq_ignore_ascii_case(b"DOC") {
            let mut digits = 0usize;
            while digits < 3
                && index + 3 + digits < bytes.len()
                && bytes[index + 3 + digits].is_ascii_digit()
            {
                digits += 1;
            }
            if digits > 0 {
                let suffix = &text[index + 3..index + 3 + digits];
                codes.push(format!("DOC{suffix}"));
                index += 3 + digits;
                continue;
            }
        }
        index += 1;
    }

    codes
}

fn split_noqa_reason(text: &str) -> (&str, Option<&str>) {
    for (index, _) in text.match_indices("--") {
        let before = text[..index].chars().next_back();
        let after = text[index + 2..].chars().next();
        let separated_before = before.is_some_and(char::is_whitespace);
        let separated_after = after.is_none_or(char::is_whitespace);
        if separated_before && separated_after {
            return (&text[..index], Some(text[index + 2..].trim()));
        }
    }
    (text, None)
}

fn noqa_remainder(text: &str) -> Option<&str> {
    for (index, _) in text.char_indices() {
        let Some(candidate) = text.get(index..index + 4) else {
            continue;
        };
        if !candidate.eq_ignore_ascii_case("noqa") {
            continue;
        }
        let previous_is_word = text[..index]
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_');
        let next_is_word = text[index + 4..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_');
        if !previous_is_word && !next_is_word {
            return Some(&text[index + 4..]);
        }
    }
    None
}

pub fn unused_selector_replacement(
    source_line: &str,
    suppression: &Suppression,
    selector_index: Option<usize>,
) -> Option<String> {
    let segment_start = source_line.rfind(&suppression.text)?;
    let replacement_segment = match selector_index {
        None => String::new(),
        Some(index) => {
            remove_selector_from_segment(&suppression.text, suppression.codes.get(index)?)
        }
    };

    let mut result = String::with_capacity(source_line.len());
    result.push_str(&source_line[..segment_start]);
    result.push_str(&replacement_segment);
    result.push_str(&source_line[segment_start + suppression.text.len()..]);
    if replacement_segment.is_empty() {
        result = result.trim_end().to_string();
    }
    Some(result)
}

fn remove_selector_from_segment(segment: &str, target: &str) -> String {
    let target = target.trim().to_ascii_uppercase();
    let lower = segment.to_ascii_lowercase();

    if let Some(noqa_pos) = lower.find("noqa") {
        if let Some(relative_colon) = segment[noqa_pos..].find(':') {
            let colon = noqa_pos + relative_colon;
            let prefix = &segment[..colon];
            let (selector_text, reason) = split_noqa_reason(&segment[colon + 1..]);
            let kept = parse_csv_codes(selector_text)
                .into_iter()
                .filter(|item| item != &target)
                .collect::<Vec<_>>();
            return if kept.is_empty() {
                String::new()
            } else {
                let mut replacement = format!("{prefix}: {}", kept.join(", "));
                if let Some(reason) = reason {
                    replacement.push_str(" --");
                    if !reason.is_empty() {
                        replacement.push(' ');
                        replacement.push_str(reason);
                    }
                }
                replacement
            };
        }
    }

    for keyword in ["ignore", "disable"] {
        if let Some(pos) = lower.find(keyword) {
            let prefix_end = pos + keyword.len();
            let prefix = segment[..prefix_end].trim_end();
            let rest = segment[prefix_end..]
                .trim_start_matches(|ch: char| ch == ':' || ch == '=' || ch.is_ascii_whitespace());
            let kept = parse_csv_codes(rest)
                .into_iter()
                .filter(|selector| selector != &target)
                .collect::<Vec<_>>();
            return if kept.is_empty() {
                String::new()
            } else {
                format!("{prefix} {}", kept.join(", "))
            };
        }
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_sk901_noqa_on_multiline_statement_boundary_gets_statement_scope() {
        for source in [
            "value = Vector3(  # noqa: SK901 -- vector\n    11,\n    12,\n)\n",
            "value = Vector3(\n    11,\n    12,\n)  # noqa: SK901 -- vector\n",
        ] {
            let state = SuppressionState::parse(source);
            let suppression = state.suppressions.first().expect("SK901 suppression");
            assert_eq!(suppression.statement_scope, Some((1, 4)));
            assert!(!state.suppressing_ids_for(2, "SK901", None).is_empty());
            assert!(!state.suppressing_ids_for(3, "SK901", None).is_empty());
            assert!(state.suppressing_ids_for(2, "SK401", None).is_empty());
        }
    }

    #[test]
    fn sk901_noqa_on_middle_continuation_line_stays_line_local() {
        let state = SuppressionState::parse(
            "value = Vector3(\n    11,  # noqa: SK901 -- one component\n    12,\n)\n",
        );
        let suppression = state.suppressions.first().expect("SK901 suppression");
        assert_eq!(suppression.statement_scope, None);
        assert!(!state.suppressing_ids_for(2, "SK901", None).is_empty());
        assert!(state.suppressing_ids_for(3, "SK901", None).is_empty());
    }

    #[test]
    fn compound_statement_header_never_creates_sk901_statement_scope() {
        let state = SuppressionState::parse(
            "def check():  # noqa: SK901 -- line only\n    return runtime_call(99)\n",
        );
        let suppression = state.suppressions.first().expect("SK901 suppression");
        assert_eq!(suppression.statement_scope, None);
        assert!(state.suppressing_ids_for(2, "SK901", None).is_empty());
    }

    #[test]
    fn remover_handles_space_separated_noqa_selectors() {
        assert_eq!(
            remove_selector_from_segment("# noqa: RUF100 DOC601 DOC603", "DOC601"),
            "# noqa: RUF100, DOC603"
        );
        assert_eq!(
            remove_selector_from_segment("# noqa: DOC601 DOC603", "DOC601"),
            "# noqa: DOC603"
        );
    }

    #[test]
    fn noqa_reason_is_not_parsed_as_selectors() {
        let state = SuppressionState::parse(
            "x=1  # noqa: SK507 -- SK999 protocol __getattr__ must raise AttributeError\n",
        );
        assert_eq!(state.suppressions.len(), 1);
        assert_eq!(state.suppressions[0].codes, vec!["SK507"]);
    }

    #[test]
    fn unused_noqa_selector_removal_preserves_reason_only_when_selectors_remain() {
        assert_eq!(
            remove_selector_from_segment("# noqa: SK507 -- reason", "SK507"),
            ""
        );
        for segment in [
            "# noqa: SK506, BLE001, S110 -- reason",
            "# noqa: BLE001, SK506, S110 -- reason",
            "# noqa: BLE001, S110, SK506 -- reason",
        ] {
            assert_eq!(
                remove_selector_from_segment(segment, "SK506"),
                "# noqa: BLE001, S110 -- reason"
            );
        }
    }

    #[test]
    fn noqa_text_inside_python_string_is_not_treated_as_comment() {
        let state = SuppressionState::parse(
            "value = \"# noqa: SK401\"\nother = '''# noqa: DOC101\ncontinued'''\n",
        );
        assert!(state.suppressions.is_empty());
    }

    #[test]
    fn noqa_after_string_literal_is_still_a_real_comment() {
        let state = SuppressionState::parse("value = \"# text\"  # noqa: SK401\n");
        assert_eq!(state.suppressions.len(), 1);
        assert_eq!(state.suppressions[0].codes, vec!["SK401"]);
    }

    #[test]
    fn native_doc_code_regex_quirks_match_upstream() {
        let state = SuppressionState::parse(
            "def f():  # noqa: xDOC101 chatter DOC1234 and doc7 DOC101\n    pass\n",
        );
        assert_eq!(state.suppressions.len(), 1);
        assert_eq!(
            state.suppressions[0].codes,
            vec!["DOC101", "DOC123", "DOC7"]
        );
        assert!(!state.suppressing_ids_for(1, "SKD101", None).is_empty());
        assert!(!state.suppressing_ids_for(1, "SKD123", None).is_empty());
        assert!(state.suppressing_ids_for(1, "SKD007", None).is_empty());
    }

    #[test]
    fn noqa_can_follow_explanatory_text_like_upstream_pydoclint() {
        let state = SuppressionState::parse(
            "def f():  # explanation noqa: doc101, doc103, F401 trailing words\n    pass\n",
        );
        assert_eq!(state.suppressions.len(), 1);
        assert_eq!(state.suppressions[0].codes, vec!["DOC101", "DOC103"]);
        assert!(!state.suppressing_ids_for(1, "SKD101", None).is_empty());
        assert!(!state.suppressing_ids_for(1, "SKD103", None).is_empty());
    }

    #[test]
    fn noqa_word_detection_rejects_embedded_noqa_text() {
        let state = SuppressionState::parse("x = 1  # xnoqa: SK401\n");
        assert!(state.suppressions.is_empty());
    }

    #[test]
    fn noqa_consumes_doc_aliases() {
        let state = SuppressionState::parse("def f():  # noqa: DOC203, E501, SKD105\n    pass\n");
        assert_eq!(state.suppressions.len(), 1);
        assert_eq!(state.suppressions[0].codes, vec!["DOC203", "SKD105"]);
        assert!(!state.suppressing_ids_for(1, "SKD203", None).is_empty());
        assert!(!state.suppressing_ids_for(1, "SKD105", None).is_empty());
    }

    #[test]
    fn noqa_leaves_foreign_t201_to_ruff() {
        let state = SuppressionState::parse("print(1)  # noqa: E501, T201, SK001\n");
        assert_eq!(state.suppressions.len(), 1);
        assert_eq!(state.suppressions[0].codes, vec!["SK001"]);
        assert!(state.suppressing_ids_for(1, "SK201", None).is_empty());
    }

    #[test]
    fn bare_noqa_is_ignored() {
        let state = SuppressionState::parse("x = 1  # noqa\n");
        assert!(state.suppressions.is_empty());
    }

    #[test]
    fn responsibility_prefers_more_specific_selector_in_same_noqa() {
        let mut state = SuppressionState::parse("x=1  # noqa: SK4, SK401\n");
        let responsible = state
            .responsible_match_for(1, "SK401", None)
            .expect("suppressed");
        assert_eq!(responsible.selector_index, Some(1));
        state.mark_match(responsible);
        assert!(!state.selector_is_used(0, Some(0)));
        assert!(state.selector_is_used(0, Some(1)));
    }

    #[test]
    fn catch_all_disable_can_partially_enable_one_rule() {
        let state =
            SuppressionState::parse("# sklint: disable\nx=1\n# sklint: enable SK401\ny=2\n");
        assert!(state.suppressing_ids_for(2, "SK401", None).len() == 1);
        assert!(state.suppressing_ids_for(4, "SK401", None).is_empty());
        assert!(state.suppressing_ids_for(4, "SK402", None).len() == 1);
    }

    #[test]
    fn unused_noqa_selector_fix_preserves_foreign_codes() {
        let state = SuppressionState::parse("x=1  # noqa: E501, SK4, SK401\n");
        let suppression = &state.suppressions[0];
        let replacement =
            unused_selector_replacement("x=1  # noqa: E501, SK4, SK401", suppression, Some(0))
                .expect("replacement");
        assert_eq!(replacement, "x=1  # noqa: E501, SK401");
    }

    #[test]
    fn unused_catch_all_fix_preserves_regular_comment() {
        let state = SuppressionState::parse("x=1  # reason  # sklint: ignore\n");
        let suppression = &state.suppressions[0];
        let replacement =
            unused_selector_replacement("x=1  # reason  # sklint: ignore", suppression, None)
                .expect("replacement");
        assert_eq!(replacement, "x=1  # reason");
    }

    #[test]
    fn foreign_t201_block_selector_does_not_control_sk201() {
        let state = SuppressionState::parse(
            "# sklint: disable T201\nprint(1)\n# sklint: enable SK201\nprint(2)\n",
        );
        assert!(state.suppressing_ids_for(2, "SK201", None).is_empty());
        assert!(state.suppressing_ids_for(4, "SK201", None).is_empty());
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
    fn noqa_text_inside_docstring_is_not_a_python_comment() {
        let state = SuppressionState::parse(
            r#"def f():
    """
    описание.  # noqa: SK617
    """
    pass
"#,
        );
        assert!(state.suppressions.is_empty());
    }
}
