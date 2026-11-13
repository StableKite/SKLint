use crate::config::EffectiveConfig;
use crate::dataclass_model::DataclassModel;
use crate::diagnostic::{Diagnostic, Span};
use crate::python_ast::PythonAst;
use rustpython_ast::Visitor;
use rustpython_parser::ast::{self, Expr};
use std::collections::{HashMap, HashSet};
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

#[derive(Clone, Copy)]
struct SelfOwner {
    class_line: usize,
}

type AssignmentOwners = HashMap<(usize, usize), SelfOwner>;

/// Function-local bindings apply to the whole function, including earlier uses.
/// A nested scope's bindings do not shadow a captured receiver in its parent.
#[derive(Default)]
struct SelfBindings {
    local: bool,
    global: bool,
    nonlocal: bool,
}

impl Visitor for SelfBindings {
    fn visit_expr_name(&mut self, node: ast::ExprName) {
        if node.id.as_str() == "self"
            && matches!(node.ctx, ast::ExprContext::Store | ast::ExprContext::Del)
        {
            self.local = true;
        }
    }

    fn visit_stmt_function_def(&mut self, node: ast::StmtFunctionDef) {
        self.local |= node.name.as_str() == "self";
    }

    fn visit_stmt_async_function_def(&mut self, node: ast::StmtAsyncFunctionDef) {
        self.local |= node.name.as_str() == "self";
    }

    fn visit_stmt_class_def(&mut self, node: ast::StmtClassDef) {
        self.local |= node.name.as_str() == "self";
    }

    fn visit_expr_lambda(&mut self, _node: ast::ExprLambda) {}

    fn visit_stmt_import(&mut self, node: ast::StmtImport) {
        self.local |= node.names.iter().any(|alias| {
            alias
                .asname
                .as_ref()
                .map(|name| name.as_str())
                .unwrap_or_else(|| alias.name.as_str().split('.').next().unwrap_or(""))
                == "self"
        });
    }

    fn visit_stmt_import_from(&mut self, node: ast::StmtImportFrom) {
        self.local |= node
            .names
            .iter()
            .any(|alias| alias.asname.as_ref().unwrap_or(&alias.name).as_str() == "self");
    }

    fn visit_excepthandler_except_handler(&mut self, node: ast::ExceptHandlerExceptHandler) {
        self.local |= node
            .name
            .as_ref()
            .is_some_and(|name| name.as_str() == "self");
        self.generic_visit_excepthandler_except_handler(node);
    }

    fn visit_pattern_match_as(&mut self, node: ast::PatternMatchAs) {
        self.local |= node
            .name
            .as_ref()
            .is_some_and(|name| name.as_str() == "self");
        self.generic_visit_pattern_match_as(node);
    }

    fn visit_pattern_match_star(&mut self, node: ast::PatternMatchStar) {
        self.local |= node
            .name
            .as_ref()
            .is_some_and(|name| name.as_str() == "self");
    }

    fn visit_pattern_match_mapping(&mut self, node: ast::PatternMatchMapping) {
        self.local |= node
            .rest
            .as_ref()
            .is_some_and(|name| name.as_str() == "self");
        self.generic_visit_pattern_match_mapping(node);
    }

    fn visit_comprehension(&mut self, node: ast::Comprehension) {
        // The iteration target is local to the comprehension, not the function.
        self.visit_expr(node.iter);
        for condition in node.ifs {
            self.visit_expr(condition);
        }
    }

    fn visit_stmt_global(&mut self, node: ast::StmtGlobal) {
        self.global |= node.names.iter().any(|name| name.as_str() == "self");
    }

    fn visit_stmt_nonlocal(&mut self, node: ast::StmtNonlocal) {
        self.nonlocal |= node.names.iter().any(|name| name.as_str() == "self");
    }
}

fn has_self_parameter(args: &ast::Arguments) -> bool {
    args.posonlyargs
        .iter()
        .chain(&args.args)
        .chain(&args.kwonlyargs)
        .any(|arg| arg.def.arg.as_str() == "self")
        || args
            .vararg
            .as_ref()
            .is_some_and(|arg| arg.arg.as_str() == "self")
        || args
            .kwarg
            .as_ref()
            .is_some_and(|arg| arg.arg.as_str() == "self")
}

struct OwnershipVisitor<'a> {
    ast: &'a PythonAst,
    lines: &'a [&'a str],
    class_body: Option<usize>,
    receiver: Option<SelfOwner>,
    assignments: AssignmentOwners,
}

impl OwnershipVisitor<'_> {
    fn function(&mut self, args: &ast::Arguments, body: Vec<ast::Stmt>) {
        let previous_class = self.class_body.take();
        let previous_receiver = self.receiver;
        let mut bindings = SelfBindings::default();
        for stmt in &body {
            bindings.visit_stmt(stmt.clone());
        }
        self.receiver = if has_self_parameter(args) {
            previous_class.map(|class_line| SelfOwner { class_line })
        } else if bindings.global || (bindings.local && !bindings.nonlocal) {
            None
        } else {
            previous_receiver
        };
        for stmt in body {
            self.visit_stmt(stmt);
        }
        self.receiver = previous_receiver;
        self.class_body = previous_class;
    }
}

impl Visitor for OwnershipVisitor<'_> {
    fn visit_stmt_class_def(&mut self, node: ast::StmtClassDef) {
        let previous = self.class_body.replace(self.ast.location_of(&node).line);
        for stmt in node.body {
            self.visit_stmt(stmt);
        }
        self.class_body = previous;
    }

    fn visit_stmt_function_def(&mut self, node: ast::StmtFunctionDef) {
        self.function(&node.args, node.body);
    }

    fn visit_stmt_async_function_def(&mut self, node: ast::StmtAsyncFunctionDef) {
        self.function(&node.args, node.body);
    }

    fn visit_expr_lambda(&mut self, node: ast::ExprLambda) {
        let previous = self.receiver;
        if has_self_parameter(&node.args) {
            self.receiver = None;
        }
        self.visit_expr(*node.body);
        self.receiver = previous;
    }

    fn visit_expr_attribute(&mut self, node: ast::ExprAttribute) {
        if matches!(node.ctx, ast::ExprContext::Store)
            && matches!(node.value.as_ref(), Expr::Name(name) if name.id.as_str() == "self")
        {
            if let Some(owner) = self.receiver {
                let end = self.ast.end_location_of(&node);
                if let Some(line) = self.lines.get(end.line - 1) {
                    let attr_start = end.column - 1 - node.attr.as_str().len();
                    let column = line[..attr_start].chars().count() + 1;
                    self.assignments.insert((end.line, column), owner);
                }
            }
        }
        self.generic_visit_expr_attribute(node);
    }
}

fn assignment_owners(ast: &PythonAst, lines: &[&str]) -> AssignmentOwners {
    let mut visitor = OwnershipVisitor {
        ast,
        lines,
        class_body: None,
        receiver: None,
        assignments: HashMap::new(),
    };
    for stmt in &ast.suite {
        visitor.visit_stmt(stmt.clone());
    }
    visitor.assignments
}

fn assignment_belongs_to(
    owners: Option<&AssignmentOwners>,
    class_line: usize,
    line: usize,
    column: usize,
) -> bool {
    owners.is_none_or(|owners| {
        owners
            .get(&(line, column))
            .is_some_and(|owner| owner.class_line == class_line)
    })
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
    let ast = PythonAst::parse(source, &display_path).ok();
    let owners = ast.as_ref().map(|ast| assignment_owners(ast, &lines));
    let classes = collect_classes(&lines, owners.as_ref());
    let project_declared = ast
        .as_ref()
        .map(|ast| DataclassModel::from_path(path, source, ast))
        .map(|model| {
            model
                .classes
                .iter()
                .map(|class| (class.line, model.effective_declared_attrs(&class.name)))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let mut diagnostics = Vec::new();

    if sk701_enabled {
        run_self_dynamic_attribute_rule(
            &display_path,
            &lines,
            &classes,
            owners.as_ref(),
            &project_declared,
            &mut diagnostics,
        );
    }
    if sk702_enabled {
        run_known_dynamic_object_attribute_rule(
            &display_path,
            &lines,
            &classes,
            &project_declared,
            &mut diagnostics,
        );
    }

    diagnostics
}

fn run_self_dynamic_attribute_rule(
    display_path: &str,
    lines: &[&str],
    classes: &[ClassInfo],
    owners: Option<&AssignmentOwners>,
    project_declared: &HashMap<usize, HashSet<String>>,
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
                    if !assignment_belongs_to(owners, class.start_line, line_no, start_col) {
                        continue;
                    }
                    if class_declares_attr(classes, class_idx, &attr, project_declared) {
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
    project_declared: &HashMap<usize, HashSet<String>>,
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
                if class_declares_attr(classes, variable.class_idx, &attr, project_declared) {
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

fn class_declares_attr(
    classes: &[ClassInfo],
    class_idx: usize,
    attr: &str,
    project_declared: &HashMap<usize, HashSet<String>>,
) -> bool {
    let class = &classes[class_idx];
    project_declared
        .get(&class.start_line)
        .is_some_and(|attrs| attrs.contains(attr))
        || class.declared_attrs.contains(attr)
        || class.bases.iter().any(|base| {
            resolve_base_class(classes, class_idx, simple_name(base)).is_some_and(|base_idx| {
                class_declares_attr(classes, base_idx, attr, project_declared)
            })
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

fn collect_classes(lines: &[&str], owners: Option<&AssignmentOwners>) -> Vec<ClassInfo> {
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
        let declared_attrs = collect_declared_attrs(
            lines,
            class_start + 1,
            class_end,
            class_indent,
            &methods,
            owners,
        );

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
    owners: Option<&AssignmentOwners>,
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
            for (attr, column, _) in self_attribute_assignment_occurrences(code_part) {
                if assignment_belongs_to(owners, start_line - 1, line_no, column) {
                    attrs.insert(attr);
                }
            }
        }
    }

    attrs
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
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn nested_initializer_belongs_to_inner_not_outer() {
        let source = "class Outer:\n    def make(self):\n        class Inner:\n            _event_owner: int\n            def __init__(self, owner):\n                self._event_owner = owner\n        return Inner(1)\n\nclass Invalid:\n    def set_value(self, owner):\n        self._undeclared = owner\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 11);
        assert!(diagnostics[0].message.contains("in `Invalid`"));
    }

    #[test]
    fn nested_initializer_does_not_declare_an_outer_field() {
        let source = "class Outer:\n    def __init__(self):\n        class Inner:\n            def __init__(self):\n                self.shared = 1\n    def update(self):\n        self.shared = 2\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 7);
        assert!(diagnostics[0].message.contains("in `Outer`"));
    }

    #[test]
    fn nested_methods_report_only_their_actual_class() {
        let source = "class Outer:\n    def make(self):\n        class Inner:\n            def update(self):\n                self.dynamic = 1\n        return Inner()\n    def update(self):\n        self.other = 2\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics
            .iter()
            .any(|d| d.line == 5 && d.message.contains("in `Inner`")));
        assert!(diagnostics
            .iter()
            .any(|d| d.line == 8 && d.message.contains("in `Outer`")));
    }

    #[test]
    fn sibling_nested_classes_keep_separate_declarations() {
        let source = "class Outer:\n    def make(self):\n        class First:\n            value: int\n            def update(self):\n                self.value = 1\n        class Second:\n            def update(self):\n                self.value = 2\n        return First(), Second()\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 9);
        assert!(diagnostics[0].message.contains("in `Second`"));
    }

    #[test]
    fn deep_nested_classes_preserve_positive_and_negative_owners() {
        let source = "class Outer:\n    def make(self):\n        class Middle:\n            def make(self):\n                class Inner:\n                    def __init__(self):\n                        self.valid = 1\n                    def update(self):\n                        self.invalid = 2\n                return Inner()\n        return Middle()\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 9);
        assert!(diagnostics[0].message.contains("in `Inner`"));
    }

    #[test]
    fn closure_capturing_self_still_reports_the_enclosing_class() {
        let source = "class Outer:\n    def make(self):\n        def update():\n            self.dynamic = 1\n        return update\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 4);
        assert!(diagnostics[0].message.contains("in `Outer`"));
    }

    #[test]
    fn closure_parameter_self_does_not_belong_to_outer() {
        let source = "class Outer:\n    def make(self):\n        def update(self):\n            self.foreign = 1\n        self.actual = 2\n        return update\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 5);
        assert!(diagnostics[0].message.contains("`actual`"));
    }

    #[test]
    fn closure_local_self_shadows_even_before_its_assignment() {
        let source = "class Outer:\n    def make(self):\n        def update(value):\n            self.foreign = 1\n            self = value\n        self.actual = 2\n        return update\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 6);
    }

    #[test]
    fn closure_nonlocal_self_retains_the_enclosing_owner() {
        let source = "class Outer:\n    def make(self):\n        def update():\n            nonlocal self\n            self.dynamic = 1\n        return update\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 5);
    }

    #[test]
    fn closure_global_self_does_not_belong_to_outer() {
        let source = "class Outer:\n    def make(self):\n        def update():\n            global self\n            self.foreign = 1\n        self.actual = 2\n        return update\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 6);
    }

    #[test]
    fn nested_same_name_does_not_inherit_an_unrelated_top_level_declaration() {
        let source = "class Inner:\n    shared: int\n\nclass Outer:\n    def make(self):\n        class Inner:\n            def update(self):\n                self.shared = 1\n        return Inner()\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 8);
        assert!(diagnostics[0].message.contains("in `Inner`"));
    }

    #[test]
    fn unicode_receiver_assignment_columns_retain_character_coordinates() {
        let source = "class Outer:\n    def update(self):\n        café = 1; self.поле = café\n";
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, 3);
        assert_eq!(diagnostics[0].column, 24);
    }

    #[test]
    fn type_checking_class_annotations_count_as_declarations_for_sk701() {
        let source = r#"from typing import TYPE_CHECKING

class C:
    if TYPE_CHECKING:
        flag: bool

    def run(self) -> None:
        self.flag = True
"#;
        let diagnostics = run_dynamic_attribute_rules(Path::new("example.py"), source, &config());
        assert!(!diagnostics.iter().any(|diag| diag.code == "SK701"));
    }

    #[test]
    fn ignores_attributes_declared_in_imported_base_initializer() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sklint-dynamic-inherited-{unique}"));
        let pkg = root.join("pkg");
        fs::create_dir_all(&pkg).expect("create package");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.sklint]\nstrict = true\n",
        )
        .expect("write pyproject");
        fs::write(pkg.join("__init__.py"), "").expect("write init");
        fs::write(
            pkg.join("base.py"),
            "class Base:\n    def __init__(self):\n        self.is_reading = False\n        self._is_open = False\n",
        )
        .expect("write base");
        let child_path = pkg.join("child.py");
        let child_source = "from .base import Base\n\nclass Child(Base):\n    def start(self):\n        self.is_reading = True\n        self._is_open = True\n        self.new_state = 1\n";
        fs::write(&child_path, child_source).expect("write child");

        let diagnostics = run_dynamic_attribute_rules(&child_path, child_source, &config());
        assert!(!diagnostics
            .iter()
            .any(|diag| diag.message.contains("is_reading")));
        assert!(!diagnostics
            .iter()
            .any(|diag| diag.message.contains("_is_open")));
        assert!(diagnostics
            .iter()
            .any(|diag| diag.message.contains("new_state")));
        let _ = fs::remove_dir_all(root);
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
