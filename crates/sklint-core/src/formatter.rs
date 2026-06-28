use crate::analyzer::{analyze, AnalysisInput};
use crate::config::VscodeConfig;
use crate::diagnostic::{Diagnostic, Fix};
use crate::identifier::is_identifier_continue;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatReport {
    pub source: String,
    pub applied: usize,
    /// Safe diagnostics that still remain after formatting.
    /// A non-zero value means the formatter could not reach a clean fixed point.
    pub remaining_safe: usize,
}

pub fn format_source(path: PathBuf, source: String, vscode_config: VscodeConfig) -> FormatReport {
    let mut current = source;
    let mut total_applied = 0usize;
    let mut seen = HashSet::new();

    // Structural moves are batched by category, but later categories and
    // newly exposed diagnostics still require a fresh analysis round. Keep a
    // source-sized safety bound so valid long dependency chains can converge.
    let max_rounds = current.lines().count().max(32);
    for _ in 0..max_rounds {
        if !seen.insert(current.clone()) {
            break;
        }

        let report = analyze(AnalysisInput {
            path: path.clone(),
            source: current.clone(),
            vscode_config: vscode_config.clone(),
        });

        // Structural ordering fixes must be driven only by diagnostics that
        // survived project config and local suppression filtering. This keeps
        // ignore/noqa/block directives authoritative for format and --fix.
        let order_report = apply_ordering_fixes(&current, &report.diagnostics);
        if order_report.applied > 0 {
            current = order_report.source;
            total_applied += order_report.applied;
            continue;
        }

        let mut fixes: Vec<Fix> = report
            .diagnostics
            .into_iter()
            .filter_map(|diag| diag.fix)
            .filter(|fix| fix.safe)
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

        let index = SourceLineIndex::new(&current);
        let mut prepared = Vec::new();
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
            let Some(start) = index.byte_offset(&current, fix.start_line, fix.start_column) else {
                continue;
            };
            let Some(end) = index.byte_offset(&current, fix.end_line, fix.end_column) else {
                continue;
            };
            if start > end || end > current.len() || current[start..end] == fix.replacement {
                continue;
            }
            applied_ranges.push(range);
            prepared.push((start, end, fix.replacement));
        }

        let applied_this_round = prepared.len();
        for (start, end, replacement) in prepared {
            current.replace_range(start..end, &replacement);
        }
        total_applied += applied_this_round;
        if applied_this_round == 0 {
            break;
        }
    }

    let remaining_safe = analyze(AnalysisInput {
        path,
        source: current.clone(),
        vscode_config,
    })
    .diagnostics
    .iter()
    .filter(|diagnostic| diagnostic.fix.as_ref().is_some_and(|fix| fix.safe))
    .count();

    FormatReport {
        source: current,
        applied: total_applied,
        remaining_safe,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceLineIndex {
    starts: Vec<usize>,
}

impl SourceLineIndex {
    fn new(source: &str) -> Self {
        let mut starts = vec![0usize];
        starts.extend(
            source
                .bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(idx, _)| idx + 1),
        );
        Self { starts }
    }

    fn byte_offset(&self, source: &str, line: usize, column: usize) -> Option<usize> {
        if line == 0 || column == 0 {
            return None;
        }
        if line == self.starts.len() + 1 && column == 1 {
            return Some(source.len());
        }
        let start = *self.starts.get(line - 1)?;
        let end = self
            .starts
            .get(line)
            .map(|next| next.saturating_sub(1))
            .unwrap_or(source.len());
        let logical_line = source.get(start..end)?;
        if column == logical_line.chars().count() + 1 {
            return Some(end);
        }
        logical_line
            .char_indices()
            .nth(column - 1)
            .map(|(byte_idx, _)| start + byte_idx)
    }
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

fn apply_ordering_fixes(source: &str, diagnostics: &[Diagnostic]) -> FormatReport {
    let sk505_lines: HashSet<usize> = diagnostics
        .iter()
        .filter(|diag| diag.code == "SK505")
        .map(|diag| diag.line)
        .collect();
    let sk509_lines: HashSet<usize> = diagnostics
        .iter()
        .filter(|diag| diag.code == "SK509")
        .map(|diag| diag.line)
        .collect();

    if sk505_lines.is_empty() && sk509_lines.is_empty() {
        return FormatReport {
            source: source.to_string(),
            applied: 0,
            remaining_safe: 0,
        };
    }

    let lines = format_lines(source);
    let defs = parse_format_defs(&lines);
    let mut buffer = OriginalLineBuffer::new(source);

    // Keep the historical category priority, but apply every independent move
    // from the selected category in one pass. The outer fixed-point loop will
    // re-analyze before proceeding to the next category, preserving suppression
    // semantics when one structural move exposes a new diagnostic.
    let mut applied = reorder_special_methods_batch(&mut buffer, &defs, &sk509_lines);
    if applied == 0 {
        applied = reorder_method_dependencies_batch(&mut buffer, &lines, &defs, &sk505_lines);
    }
    if applied == 0 {
        applied = reorder_top_level_definitions_batch(&mut buffer, &lines, &defs, &sk505_lines);
    }

    FormatReport {
        source: if applied == 0 {
            source.to_string()
        } else {
            buffer.into_source()
        },
        applied,
        remaining_safe: 0,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OriginalLine {
    original_no: usize,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OriginalLineBuffer {
    lines: Vec<OriginalLine>,
    had_final_newline: bool,
}

impl OriginalLineBuffer {
    fn new(source: &str) -> Self {
        Self {
            lines: split_preserving_logical_lines(source)
                .into_iter()
                .enumerate()
                .map(|(idx, text)| OriginalLine {
                    original_no: idx + 1,
                    text,
                })
                .collect(),
            had_final_newline: source.ends_with('\n'),
        }
    }

    fn move_original_range_before(&mut self, start: usize, end: usize, destination: usize) -> bool {
        if start == 0 || end < start || destination == 0 || (start..=end).contains(&destination) {
            return false;
        }

        let Some(start_idx) = self.lines.iter().position(|line| line.original_no == start) else {
            return false;
        };
        let Some(end_idx) = self.lines.iter().position(|line| line.original_no == end) else {
            return false;
        };
        let Some(destination_idx) = self
            .lines
            .iter()
            .position(|line| line.original_no == destination)
        else {
            return false;
        };
        if start_idx > end_idx || (start_idx..=end_idx).contains(&destination_idx) {
            return false;
        }

        let block: Vec<OriginalLine> = self.lines.drain(start_idx..=end_idx).collect();
        let insert_idx = if destination_idx > end_idx {
            destination_idx - block.len()
        } else {
            destination_idx
        };
        self.lines.splice(insert_idx..insert_idx, block);
        true
    }

    fn into_source(mut self) -> String {
        while self
            .lines
            .last()
            .is_some_and(|line| line.text.trim().is_empty())
        {
            self.lines.pop();
        }
        let logical_lines: Vec<String> = self.lines.into_iter().map(|line| line.text).collect();
        join_logical_lines(&logical_lines, self.had_final_newline)
    }
}

fn reorder_special_methods_batch(
    buffer: &mut OriginalLineBuffer,
    defs: &[FormatDef],
    target_lines: &HashSet<usize>,
) -> usize {
    let mut applied = 0usize;

    for class_idx in defs
        .iter()
        .enumerate()
        .filter(|(_, def)| def.kind == FormatDefKind::Class)
        .map(|(idx, _)| idx)
    {
        let mut methods: Vec<usize> = defs
            .iter()
            .enumerate()
            .filter(|(_, def)| def.kind == FormatDefKind::Function && def.parent == Some(class_idx))
            .map(|(idx, _)| idx)
            .collect();
        methods.sort_by_key(|method_idx| defs[*method_idx].start);
        let targets: Vec<usize> = methods
            .iter()
            .copied()
            .filter(|method_idx| target_lines.contains(&defs[*method_idx].start))
            .collect();

        for method_idx in targets {
            let Some(current_pos) = methods
                .iter()
                .position(|candidate| *candidate == method_idx)
            else {
                continue;
            };
            let phase = special_method_format_phase(&defs[method_idx].name);
            let Some(destination_pos) = methods[..current_pos]
                .iter()
                .position(|candidate| special_method_format_phase(&defs[*candidate].name) > phase)
            else {
                continue;
            };
            let destination_idx = methods[destination_pos];
            if buffer.move_original_range_before(
                defs[method_idx].group_start,
                defs[method_idx].end,
                defs[destination_idx].group_start,
            ) {
                methods.remove(current_pos);
                methods.insert(destination_pos, method_idx);
                applied += 1;
            }
        }
    }

    applied
}

fn reorder_method_dependencies_batch(
    buffer: &mut OriginalLineBuffer,
    lines: &[FormatLine],
    defs: &[FormatDef],
    target_lines: &HashSet<usize>,
) -> usize {
    let mut applied = 0usize;

    for class_idx in defs
        .iter()
        .enumerate()
        .filter(|(_, def)| def.kind == FormatDefKind::Class)
        .map(|(idx, _)| idx)
    {
        let mut method_indices: Vec<usize> = defs
            .iter()
            .enumerate()
            .filter(|(_, def)| def.kind == FormatDefKind::Function && def.parent == Some(class_idx))
            .map(|(idx, _)| idx)
            .collect();
        method_indices.sort_by_key(|idx| defs[*idx].start);
        if method_indices.len() < 2 {
            continue;
        }

        let graph = method_dependency_graph(lines, defs, &method_indices);
        let mut constraints = Vec::new();
        for (caller_pos, method_idx) in method_indices.iter().enumerate() {
            let method = &defs[*method_idx];
            if method.name == "__init__" || method.name == "__post_init__" {
                continue;
            }
            let body_end = method.end.min(lines.len());
            for line in &lines[method.start.saturating_sub(1)..body_end] {
                if !target_lines.contains(&line.no) {
                    continue;
                }
                for (target_pos, target_idx) in method_indices.iter().enumerate() {
                    let target = &defs[*target_idx];
                    if target.start <= method.start || target.name == method.name {
                        continue;
                    }
                    if !contains_self_method_call(&line.code, &target.name)
                        || dependency_reaches(&graph, target_pos, caller_pos)
                    {
                        continue;
                    }
                    // target must precede caller.
                    constraints.push((*target_idx, *method_idx));
                }
            }
        }
        constraints.sort_unstable();
        constraints.dedup();
        if constraints.is_empty() {
            continue;
        }

        let desired = stable_topological_order(&method_indices, &constraints);
        applied += apply_definition_order(buffer, defs, &method_indices, &desired);
    }

    applied
}

fn stable_topological_order(nodes: &[usize], constraints: &[(usize, usize)]) -> Vec<usize> {
    let positions: std::collections::HashMap<usize, usize> = nodes
        .iter()
        .copied()
        .enumerate()
        .map(|(pos, node)| (node, pos))
        .collect();
    let mut adjacency = vec![Vec::new(); nodes.len()];
    let mut indegree = vec![0usize; nodes.len()];

    for (before, after) in constraints {
        let (Some(&before_pos), Some(&after_pos)) = (positions.get(before), positions.get(after))
        else {
            continue;
        };
        if before_pos == after_pos || adjacency[before_pos].contains(&after_pos) {
            continue;
        }
        adjacency[before_pos].push(after_pos);
        indegree[after_pos] += 1;
    }

    let mut emitted = vec![false; nodes.len()];
    let mut result = Vec::with_capacity(nodes.len());
    while result.len() < nodes.len() {
        let next = (0..nodes.len()).find(|idx| !emitted[*idx] && indegree[*idx] == 0);
        let Some(next) = next else {
            // Defensive fallback. Cyclic edges are normally filtered before
            // this point; retain source order for any unexpected remainder.
            result.extend(
                nodes
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| !emitted[*idx])
                    .map(|(_, node)| *node),
            );
            break;
        };
        emitted[next] = true;
        result.push(nodes[next]);
        for dependent in &adjacency[next] {
            indegree[*dependent] = indegree[*dependent].saturating_sub(1);
        }
    }
    result
}

fn apply_definition_order(
    buffer: &mut OriginalLineBuffer,
    defs: &[FormatDef],
    current_order: &[usize],
    desired_order: &[usize],
) -> usize {
    let mut current = current_order.to_vec();
    let mut applied = 0usize;

    for desired_pos in 0..desired_order.len() {
        if current.get(desired_pos) == desired_order.get(desired_pos) {
            continue;
        }
        let desired_idx = desired_order[desired_pos];
        let Some(current_pos) = current
            .iter()
            .position(|candidate| *candidate == desired_idx)
        else {
            continue;
        };
        let destination_idx = current[desired_pos];
        if buffer.move_original_range_before(
            defs[desired_idx].group_start,
            defs[desired_idx].end,
            defs[destination_idx].group_start,
        ) {
            current.remove(current_pos);
            current.insert(desired_pos, desired_idx);
            applied += 1;
        }
    }

    applied
}

fn reorder_top_level_definitions_batch(
    buffer: &mut OriginalLineBuffer,
    lines: &[FormatLine],
    defs: &[FormatDef],
    target_lines: &HashSet<usize>,
) -> usize {
    let mut top_indices: Vec<usize> = defs
        .iter()
        .enumerate()
        .filter(|(_, def)| def.indent == 0)
        .map(|(idx, _)| idx)
        .collect();
    top_indices.sort_by_key(|idx| defs[*idx].start);
    if top_indices.len() < 2 {
        return 0;
    }

    let graph = top_level_dependency_graph(lines, defs, &top_indices);
    let mut constraints = Vec::new();
    let mut external_moves = Vec::new();

    for (target_pos, def_idx) in top_indices.iter().enumerate() {
        let def = &defs[*def_idx];
        if def.name.starts_with('_') {
            continue;
        }
        let Some(reference_line) = lines.iter().take(def.start.saturating_sub(1)).find(|line| {
            target_lines.contains(&line.no)
                && contains_top_level_format_reference(&line.code, &def.name)
        }) else {
            continue;
        };
        let source_pos = top_indices.iter().position(|candidate| {
            let candidate = &defs[*candidate];
            candidate.group_start <= reference_line.no && reference_line.no <= candidate.end
        });
        if source_pos.is_some_and(|source_pos| dependency_reaches(&graph, target_pos, source_pos)) {
            continue;
        }
        if let Some(source_pos) = source_pos {
            constraints.push((*def_idx, top_indices[source_pos]));
        } else {
            external_moves.push((*def_idx, reference_line.no));
        }
    }

    constraints.sort_unstable();
    constraints.dedup();
    let desired = stable_topological_order(&top_indices, &constraints);
    let mut applied = apply_definition_order(buffer, defs, &top_indices, &desired);

    // References in module-level statements are not nodes in the definition
    // graph. Original line identities remain stable while blocks move, so all
    // such moves can still be applied without re-parsing the source.
    for (def_idx, destination) in external_moves {
        if buffer.move_original_range_before(
            defs[def_idx].group_start,
            defs[def_idx].end,
            destination,
        ) {
            applied += 1;
        }
    }

    applied
}

fn method_dependency_graph(
    lines: &[FormatLine],
    defs: &[FormatDef],
    method_indices: &[usize],
) -> Vec<Vec<usize>> {
    let mut graph = vec![Vec::new(); method_indices.len()];
    for (source_pos, source_idx) in method_indices.iter().enumerate() {
        let source = &defs[*source_idx];
        let body_end = source.end.min(lines.len());
        for (target_pos, target_idx) in method_indices.iter().enumerate() {
            if source_pos == target_pos {
                continue;
            }
            let target = &defs[*target_idx];
            if lines[source.start.saturating_sub(1)..body_end]
                .iter()
                .any(|line| contains_self_method_call(&line.code, &target.name))
            {
                graph[source_pos].push(target_pos);
            }
        }
    }
    graph
}

fn top_level_dependency_graph(
    lines: &[FormatLine],
    defs: &[FormatDef],
    top_indices: &[usize],
) -> Vec<Vec<usize>> {
    let mut graph = vec![Vec::new(); top_indices.len()];
    for (source_pos, source_idx) in top_indices.iter().enumerate() {
        let source = &defs[*source_idx];
        let body_end = source.end.min(lines.len());
        for (target_pos, target_idx) in top_indices.iter().enumerate() {
            if source_pos == target_pos {
                continue;
            }
            let target = &defs[*target_idx];
            if lines[source.group_start.saturating_sub(1)..body_end]
                .iter()
                .any(|line| contains_top_level_format_reference(&line.code, &target.name))
            {
                graph[source_pos].push(target_pos);
            }
        }
    }
    graph
}

fn dependency_reaches(graph: &[Vec<usize>], start: usize, goal: usize) -> bool {
    let mut stack = vec![start];
    let mut visited = vec![false; graph.len()];
    while let Some(node) = stack.pop() {
        if node == goal {
            return true;
        }
        if visited[node] {
            continue;
        }
        visited[node] = true;
        stack.extend(graph[node].iter().copied());
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
        before.is_none_or(|ch| !is_identifier_continue(ch)) && matches!(after, Some('(' | '.'))
    })
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
    if start <= 1 {
        return start;
    }
    let indent = lines[start - 1].indent;
    let lower = start.saturating_sub(40).max(1);
    let mut earliest = start;
    for candidate in (lower..start).rev() {
        let line = &lines[candidate - 1];
        if line.text.trim().is_empty() || line.indent < indent {
            break;
        }
        if line.indent == indent
            && line.code.trim_start().starts_with('@')
            && valid_decorator_region(lines, candidate, start, indent)
        {
            earliest = candidate;
        }
    }
    earliest
}

fn valid_decorator_region(
    lines: &[FormatLine],
    candidate: usize,
    start: usize,
    indent: usize,
) -> bool {
    let mut line_no = candidate;
    while line_no < start {
        let line = &lines[line_no - 1];
        if line.indent != indent || !line.code.trim_start().starts_with('@') {
            return false;
        }
        let mut depth = 0usize;
        loop {
            let code = &lines[line_no - 1].code;
            for ch in code.chars() {
                match ch {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
            line_no += 1;
            if depth == 0 || line_no >= start {
                break;
            }
        }
        if depth != 0 {
            return false;
        }
    }
    line_no == start
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
        let trimmed = code.trim_end();
        if depth == 0
            && (trimmed.ends_with(':')
                || trimmed
                    .rsplit_once(':')
                    .is_some_and(|(_, body)| body.trim() == "..."))
        {
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
            _ if ch.is_ascii() => {
                out.push(ch as char);
                idx += 1;
            }
            _ => {
                let unicode = source[idx..]
                    .chars()
                    .next()
                    .expect("idx always points to a UTF-8 character boundary");
                out.push(unicode);
                idx += unicode.len_utf8();
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
    fn bulk_format_does_not_invent_dataclass_descriptions() {
        let source = "from dataclasses import dataclass\n\n@dataclass\nclass Base:\n    x: int\n\n@dataclass\nclass Child(Base):\n    \"\"\"\n    Описание\n\n    Attributes:\n        y (int): значение\n    \"\"\"\n    y: str\n";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );
        assert!(!report.source.contains("TODO: описание"));
    }

    #[test]
    fn rewrites_dataclass_attributes_when_descriptions_exist() {
        let source = "from dataclasses import dataclass\n\n@dataclass\nclass Base:\n    x: int\n\n@dataclass\nclass Child(Base):\n    \"\"\"\n    Описание\n\n    Attributes:\n        y (int): значение y\n        x (str): значение x\n    \"\"\"\n    y: str\n";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );
        assert!(report.source.contains("x (int): значение x"));
        assert!(report.source.contains("y (str): значение y"));
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
    fn batches_many_method_dependency_moves_in_one_ordering_stage() {
        let mut source = String::from("class Box:\n");
        for idx in 0..40 {
            source.push_str(&format!(
                "    def public_{idx}(self):\n        return self._helper_{idx}()\n\n"
            ));
        }
        for idx in 0..40 {
            source.push_str(&format!(
                "    def _helper_{idx}(self):\n        return {idx}\n\n"
            ));
        }

        let analysis = analyze(AnalysisInput {
            path: PathBuf::from("example.py"),
            source: source.clone(),
            vscode_config: VscodeConfig::default(),
        });
        let ordering = apply_ordering_fixes(&source, &analysis.diagnostics);

        assert_eq!(ordering.applied, 40);
        let after = analyze(AnalysisInput {
            path: PathBuf::from("example.py"),
            source: ordering.source,
            vscode_config: VscodeConfig::default(),
        });
        assert!(after.diagnostics.iter().all(|diag| diag.code != "SK505"));
    }

    #[test]
    fn batches_special_method_moves_in_one_ordering_stage() {
        let source = "class Box:\n    def helper(self):\n        return 1\n\n    def __post_init__(self):\n        pass\n\n    def __new__(cls):\n        return super().__new__(cls)\n\n    def __init__(self):\n        pass\n";
        let analysis = analyze(AnalysisInput {
            path: PathBuf::from("example.py"),
            source: source.to_string(),
            vscode_config: VscodeConfig::default(),
        });
        let ordering = apply_ordering_fixes(source, &analysis.diagnostics);

        assert_eq!(ordering.applied, 3);
        let after = analyze(AnalysisInput {
            path: PathBuf::from("example.py"),
            source: ordering.source,
            vscode_config: VscodeConfig::default(),
        });
        assert!(after.diagnostics.iter().all(|diag| diag.code != "SK509"));
    }

    #[test]
    fn batches_top_level_dependency_moves_in_one_ordering_stage() {
        let source = "def build_box():\n    return Box()\n\ndef build_item():\n    return Item()\n\nclass Box:\n    pass\n\nclass Item:\n    pass\n";
        let analysis = analyze(AnalysisInput {
            path: PathBuf::from("example.py"),
            source: source.to_string(),
            vscode_config: VscodeConfig::default(),
        });
        let ordering = apply_ordering_fixes(source, &analysis.diagnostics);

        assert_eq!(ordering.applied, 2);
        let after = analyze(AnalysisInput {
            path: PathBuf::from("example.py"),
            source: ordering.source,
            vscode_config: VscodeConfig::default(),
        });
        assert!(after.diagnostics.iter().all(|diag| diag.code != "SK505"));
    }

    #[test]
    fn completes_more_than_sixteen_ordering_moves_in_one_format_call() {
        let mut source = String::from("class Box:\n");
        for idx in 0..20 {
            source.push_str(&format!(
                "    def public_{idx}(self):\n        return self._helper_{idx}()\n\n"
            ));
        }
        for idx in 0..20 {
            source.push_str(&format!(
                "    def _helper_{idx}(self):\n        return {idx}\n\n"
            ));
        }

        let first = format_source(PathBuf::from("example.py"), source, VscodeConfig::default());
        let second = format_source(
            PathBuf::from("example.py"),
            first.source.clone(),
            VscodeConfig::default(),
        );

        assert_eq!(second.applied, 0);
        assert_eq!(second.source, first.source);
        for idx in 0..20 {
            assert!(
                first.source.find(&format!("def _helper_{idx}")).unwrap()
                    < first.source.find(&format!("def public_{idx}")).unwrap()
            );
        }
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
    #[test]
    fn ordering_respects_inline_ignore() {
        let source = "# sklint: ignore=SK505\\nclass Box:\\n    def public(self):\\n        return self._helper()\\n\\n    def _helper(self):\\n        return 1\\n";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );
        assert!(
            report.source.find("def public").unwrap() < report.source.find("def _helper").unwrap()
        );
    }

    #[test]
    fn ordering_preserves_multiline_decorators() {
        let source = r#"class Box:
    def public(self):
        return self._helper()

    @service.method(
        "iface",
        signature = "s"
    )
    def _helper(self):
        return 1
"#;
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );
        let decorator = report.source.find("@service.method").unwrap();
        let helper = report.source.find("def _helper").unwrap();
        let public = report.source.find("def public").unwrap();
        assert!(decorator < helper && helper < public);
        assert_eq!(report.source.matches("@service.method").count(), 1);
    }

    #[test]
    fn bulk_format_preserves_fstring_literal_equals() {
        let source = r#"def f(value: int) -> str:
    return f'{"="*50}{value}'
"#;
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );
        assert!(report.source.contains(r#"f'{"="*50}{value}'"#));
    }

    #[test]
    fn sk505_cycles_keep_stable_order_and_allow_other_safe_fixes() {
        let source = "\"\"\"\nОписание модуля\n\"\"\"\nvalue=1\n\n\ndef build() -> Box:\n    return Box()\n\n\nclass Box:\n    def clone(self) -> Box:\n        return build()";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );

        assert!(report.source.contains("Описание модуля"));
        assert!(!report.source.contains("\"\"\"\nОписание модуля"));
        assert!(report.source.contains("value = 1"));
        assert!(
            report.source.find("def build").unwrap() < report.source.find("class Box").unwrap()
        );
        assert_eq!(report.remaining_safe, 0);
    }

    #[test]
    fn sk801_does_not_inline_only_one_of_multiple_fstring_uses() {
        let source = r#"# sklint: strict
# sklint: ignore=SK804
def render(args: tuple[type[object], ...]) -> str:
    item_annotation = args[0]
    return "plain" if item_annotation is object else f"value={item_annotation}"
"#;
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );

        assert!(report.source.contains("item_annotation = args[0]"));
        assert_eq!(report.source.matches("item_annotation").count(), 3);
    }

    #[test]
    fn sk801_does_not_inline_only_one_of_multiple_multiline_uses() {
        let source = r#"# sklint: strict
# sklint: ignore=SK804
def build():
    runtime = get_runtime()
    return runtime.factory(
        history=runtime.history,
        reliability=runtime.reliability
    )
"#;
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );

        assert!(report.source.contains("runtime = get_runtime()"));
        assert_eq!(report.source.matches("runtime.").count(), 3);
    }

    #[test]
    fn sk801_inlines_a_single_use_in_a_multiline_statement() {
        let source = r#"# sklint: strict
# sklint: ignore=SK804
def build():
    runtime = get_runtime()
    return runtime.factory(
        history=history,
        reliability=reliability
    )
"#;
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );

        assert!(!report.source.contains("runtime = get_runtime()"));
        assert!(report.source.contains("return get_runtime().factory("));
    }

    #[test]
    fn sk801_does_not_match_ascii_name_inside_cyrillic_suffix_identifier() {
        let source = "# sklint: strict\n# sklint: ignore=SK804\ndef render():\n    x = get_value()\n    return xя";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );

        assert_eq!(report.source, source);
    }

    #[test]
    fn sk801_does_not_match_ascii_name_inside_cyrillic_prefix_identifier() {
        let source = "# sklint: strict\n# sklint: ignore=SK804\ndef render():\n    x = get_value()\n    return яx";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );

        assert_eq!(report.source, source);
    }

    #[test]
    fn sk801_does_not_match_ascii_name_before_combining_mark() {
        let source = "# sklint: strict\n# sklint: ignore=SK804\ndef render():\n    x = get_value()\n    return x\u{301}";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );

        assert_eq!(report.source, source);
    }

    #[test]
    fn sk801_inlines_a_single_use_unicode_temporary_name() {
        let source = "# sklint: strict\n# sklint: ignore=SK804\ndef render():\n    значение = get_value()\n    return значение";
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );

        assert!(!report.source.contains("значение ="));
        assert!(report.source.contains("return get_value()"));
    }

    #[test]
    fn sk801_parenthesizes_conditional_expression_before_primary_suffixes() {
        for (use_expression, expected) in [
            ("socket_cls(1)", "return (A if condition else B)(1)"),
            ("socket_cls.name", "return (A if condition else B).name"),
            ("socket_cls[0]", "return (A if condition else B)[0]"),
        ] {
            let source = format!(
                "# sklint: strict\n# sklint: ignore=SK804\ndef build(condition: bool):\n    socket_cls = A if condition else B\n    return {use_expression}"
            );
            let report =
                format_source(PathBuf::from("example.py"), source, VscodeConfig::default());

            assert!(!report.source.contains("socket_cls ="));
            assert!(
                report.source.contains(expected),
                "expected `{expected}` in:\n{}",
                report.source
            );
        }
    }

    #[test]
    fn sk801_parenthesizes_non_primary_expressions_in_larger_expressions() {
        for (source, expected) in [
            (
                "# sklint: strict\n# sklint: ignore=SK804\ndef calculate(left, right, scale):\n    value = left + right\n    return value * scale",
                "return (left + right) * scale",
            ),
            (
                "# sklint: strict\n# sklint: ignore=SK804\ndef calculate(base):\n    value = -base\n    return value ** 2",
                "return (-base) ** 2",
            ),
            (
                "# sklint: strict\n# sklint: ignore=SK804\ndef label():\n    value = \"left\" \"right\"\n    return value.upper()",
                "return (\"left\" \"right\").upper()",
            ),
        ] {
            let report = format_source(
                PathBuf::from("example.py"),
                source.to_string(),
                VscodeConfig::default(),
            );
            assert!(
                report.source.contains(expected),
                "expected `{expected}` in:\n{}",
                report.source
            );
        }
    }

    #[test]
    fn sk801_keeps_direct_conditional_return_and_primary_chain_clean() {
        let direct = "# sklint: strict\n# sklint: ignore=SK804\ndef choose(condition):\n    result = A if condition else B\n    return result";
        let direct_report = format_source(
            PathBuf::from("example.py"),
            direct.to_string(),
            VscodeConfig::default(),
        );
        assert!(direct_report
            .source
            .contains("return A if condition else B"));
        assert!(!direct_report
            .source
            .contains("return (A if condition else B)"));

        let primary = "# sklint: strict\n# sklint: ignore=SK804\ndef build():\n    runtime = get_runtime()\n    return runtime.factory()";
        let primary_report = format_source(
            PathBuf::from("example.py"),
            primary.to_string(),
            VscodeConfig::default(),
        );
        assert!(primary_report
            .source
            .contains("return get_runtime().factory()"));
        assert!(!primary_report
            .source
            .contains("return (get_runtime()).factory()"));
    }

    #[test]
    fn strict_bulk_fixes_keep_raw_string_expressions() {
        let source = r#"# sklint: strict
# sklint: ignore=SK804
def f() -> str:
    item_annotation = "value"
    return item_annotation

def g(flag: str) -> str:
    if flag == "x":
        return "a"
    return "b"
"#;
        let report = format_source(
            PathBuf::from("example.py"),
            source.to_string(),
            VscodeConfig::default(),
        );
        assert!(report.source.contains("return \"value\""));
        assert!(report
            .source
            .contains("return \"a\" if flag == \"x\" else \"b\""));
        assert!(!report.source.contains("return _"));
    }
}
