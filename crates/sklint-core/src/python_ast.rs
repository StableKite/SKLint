//! Shared Python syntax layer backed by `rustpython-parser`.
//!
//! New semantic rules must use this module instead of reconstructing Python
//! structure from indentation or regular expressions.  The parser yields the
//! same broad AST families as CPython (`Stmt`, `Expr`, `Arguments`, ...), with
//! byte ranges that we map back to SKLint line/column spans.

use rustpython_parser::ast::{self, Constant, Expr, Ranged, Stmt};
use rustpython_parser::{Parse, ParseError};
use std::collections::BTreeSet;

pub type Suite = ast::Suite;

#[derive(Debug, Clone)]
pub struct PythonAst {
    pub suite: Suite,
    line_starts: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FunctionFacts {
    pub has_return: bool,
    pub has_bare_return: bool,
    pub has_yield: bool,
    pub has_raise: bool,
    pub has_assert: bool,
    pub raised_exceptions: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct ExceptContext {
    types: Vec<String>,
    binding: Option<String>,
}

impl PythonAst {
    pub fn parse(source: &str, filename: &str) -> Result<Self, ParseError> {
        let suite = Suite::parse(source, filename)?;
        Ok(Self {
            suite,
            line_starts: line_starts(source),
        })
    }

    pub fn location_of_offset(&self, byte_offset: usize) -> SourceLocation {
        let line_index = match self.line_starts.binary_search(&byte_offset) {
            Ok(index) => index,
            Err(0) => 0,
            Err(index) => index - 1,
        };
        SourceLocation {
            line: line_index + 1,
            column: byte_offset.saturating_sub(self.line_starts[line_index]) + 1,
        }
    }

    pub fn location_of<T: Ranged>(&self, node: &T) -> SourceLocation {
        self.location_of_offset(text_size_to_usize(node.start()))
    }

    pub fn end_location_of<T: Ranged>(&self, node: &T) -> SourceLocation {
        self.location_of_offset(text_size_to_usize(node.end()))
    }
}

pub fn source_text<'a, T: Ranged>(source: &'a str, node: &T) -> &'a str {
    let start = text_size_to_usize(node.start()).min(source.len());
    let end = text_size_to_usize(node.end()).min(source.len());
    source.get(start..end).unwrap_or("")
}

pub fn expression_text(source: &str, expr: &Expr) -> String {
    source_text(source, expr).trim().to_string()
}

/// Return a dotted qualified name for simple names/attributes.
///
/// Calls are deliberately unwrapped so decorators such as `@dataclass()` and
/// `@typing.dataclass_transform(...)` resolve to the callable name.
pub fn qualified_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Name(node) => Some(node.id.to_string()),
        Expr::Attribute(node) => {
            let base = qualified_name(&node.value)?;
            Some(format!("{base}.{}", node.attr))
        }
        Expr::Call(node) => qualified_name(&node.func),
        _ => None,
    }
}

pub fn final_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Name(node) => Some(node.id.as_str()),
        Expr::Attribute(node) => Some(node.attr.as_str()),
        Expr::Call(node) => final_name(&node.func),
        _ => None,
    }
}

pub fn subscript_base_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Subscript(node) => qualified_name(&node.value),
        _ => None,
    }
}

pub fn string_constant(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Constant(node) => match &node.value {
            Constant::Str(value) => Some(value.as_str()),
            _ => None,
        },
        _ => None,
    }
}

pub fn suite_docstring(body: &[Stmt]) -> Option<&str> {
    let Stmt::Expr(stmt) = body.first()? else {
        return None;
    };
    string_constant(&stmt.value)
}

pub fn suite_docstring_stmt(body: &[Stmt]) -> Option<&Stmt> {
    let stmt = body.first()?;
    let Stmt::Expr(expr_stmt) = stmt else {
        return None;
    };
    string_constant(&expr_stmt.value)?;
    Some(stmt)
}

/// Collect runtime-control-flow facts from a function body.
///
/// Nested function/class definitions are skipped. This is the crucial semantic
/// distinction that text/indentation heuristics cannot model reliably and is
/// required for pydoclint parity around return/yield/raise checks.
pub fn function_facts(body: &[Stmt]) -> FunctionFacts {
    let mut facts = FunctionFacts::default();
    let mut raised = BTreeSet::new();
    walk_statements(body, &mut facts, &mut raised, None);
    facts.raised_exceptions = raised.into_iter().collect();
    facts
}

fn walk_statements(
    body: &[Stmt],
    facts: &mut FunctionFacts,
    raised: &mut BTreeSet<String>,
    active_except: Option<&ExceptContext>,
) {
    for stmt in body {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::AsyncFunctionDef(_) | Stmt::ClassDef(_) => {
                // A nested scope is not part of the enclosing function's
                // return/yield/raise behavior.
            }
            Stmt::Return(node) => {
                facts.has_return = true;
                if node.value.is_none() {
                    facts.has_bare_return = true;
                }
                if let Some(value) = &node.value {
                    walk_expr(value, facts);
                }
            }
            Stmt::Raise(node) => {
                facts.has_raise = true;
                if let Some(exc) = &node.exc {
                    if let Expr::Name(name) = exc.as_ref() {
                        if let Some(context) = active_except {
                            if context.binding.as_deref() == Some(name.id.as_str()) {
                                raised.extend(context.types.iter().cloned());
                            } else if let Some(name) = raised_exception_name(exc, active_except) {
                                raised.insert(name);
                            }
                        } else if let Some(name) = raised_exception_name(exc, active_except) {
                            raised.insert(name);
                        }
                    } else if let Some(name) = raised_exception_name(exc, active_except) {
                        raised.insert(name);
                    }
                    walk_expr(exc, facts);
                } else if let Some(context) = active_except {
                    raised.extend(context.types.iter().cloned());
                }
                if let Some(cause) = &node.cause {
                    walk_expr(cause, facts);
                }
            }
            Stmt::Assert(node) => {
                facts.has_assert = true;
                walk_expr(&node.test, facts);
                if let Some(message) = &node.msg {
                    walk_expr(message, facts);
                }
            }
            Stmt::For(node) => {
                walk_expr(&node.target, facts);
                walk_expr(&node.iter, facts);
                walk_statements(&node.body, facts, raised, active_except);
                walk_statements(&node.orelse, facts, raised, active_except);
            }
            Stmt::AsyncFor(node) => {
                walk_expr(&node.target, facts);
                walk_expr(&node.iter, facts);
                walk_statements(&node.body, facts, raised, active_except);
                walk_statements(&node.orelse, facts, raised, active_except);
            }
            Stmt::While(node) => {
                walk_expr(&node.test, facts);
                walk_statements(&node.body, facts, raised, active_except);
                walk_statements(&node.orelse, facts, raised, active_except);
            }
            Stmt::If(node) => {
                walk_expr(&node.test, facts);
                walk_statements(&node.body, facts, raised, active_except);
                walk_statements(&node.orelse, facts, raised, active_except);
            }
            Stmt::With(node) => {
                for item in &node.items {
                    walk_expr(&item.context_expr, facts);
                    if let Some(vars) = &item.optional_vars {
                        walk_expr(vars, facts);
                    }
                }
                walk_statements(&node.body, facts, raised, active_except);
            }
            Stmt::AsyncWith(node) => {
                for item in &node.items {
                    walk_expr(&item.context_expr, facts);
                    if let Some(vars) = &item.optional_vars {
                        walk_expr(vars, facts);
                    }
                }
                walk_statements(&node.body, facts, raised, active_except);
            }
            Stmt::Match(node) => {
                walk_expr(&node.subject, facts);
                for case in &node.cases {
                    if let Some(guard) = &case.guard {
                        walk_expr(guard, facts);
                    }
                    walk_statements(&case.body, facts, raised, active_except);
                }
            }
            Stmt::Try(node) => {
                walk_statements(&node.body, facts, raised, active_except);
                for handler in &node.handlers {
                    walk_except_handler(handler, facts, raised);
                }
                walk_statements(&node.orelse, facts, raised, active_except);
                walk_statements(&node.finalbody, facts, raised, active_except);
            }
            Stmt::TryStar(node) => {
                walk_statements(&node.body, facts, raised, active_except);
                for handler in &node.handlers {
                    walk_except_handler(handler, facts, raised);
                }
                walk_statements(&node.orelse, facts, raised, active_except);
                walk_statements(&node.finalbody, facts, raised, active_except);
            }
            Stmt::Expr(node) => walk_expr(&node.value, facts),
            Stmt::Assign(node) => {
                for target in &node.targets {
                    walk_expr(target, facts);
                }
                walk_expr(&node.value, facts);
            }
            Stmt::AnnAssign(node) => {
                walk_expr(&node.target, facts);
                walk_expr(&node.annotation, facts);
                if let Some(value) = &node.value {
                    walk_expr(value, facts);
                }
            }
            Stmt::AugAssign(node) => {
                walk_expr(&node.target, facts);
                walk_expr(&node.value, facts);
            }
            Stmt::Delete(node) => {
                for target in &node.targets {
                    walk_expr(target, facts);
                }
            }
            Stmt::TypeAlias(node) => {
                walk_expr(&node.name, facts);
                walk_expr(&node.value, facts);
            }
            Stmt::Import(_)
            | Stmt::ImportFrom(_)
            | Stmt::Global(_)
            | Stmt::Nonlocal(_)
            | Stmt::Pass(_)
            | Stmt::Break(_)
            | Stmt::Continue(_) => {}
        }
    }
}

fn walk_except_handler(
    handler: &ast::ExceptHandler,
    facts: &mut FunctionFacts,
    raised: &mut BTreeSet<String>,
) {
    let ast::ExceptHandler::ExceptHandler(node) = handler;
    let context = ExceptContext {
        types: node
            .type_
            .as_deref()
            .map(exception_type_names)
            .unwrap_or_default(),
        binding: node.name.as_ref().map(ToString::to_string),
    };
    if let Some(type_expr) = &node.type_ {
        walk_expr(type_expr, facts);
    }
    walk_statements(&node.body, facts, raised, Some(&context));
}

fn exception_type_names(expr: &Expr) -> Vec<String> {
    fn handler_name(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Name(node) => Some(node.id.to_string()),
            Expr::Attribute(_) => qualified_name(expr),
            _ => None,
        }
    }
    match expr {
        Expr::Tuple(node) => node.elts.iter().filter_map(handler_name).collect(),
        _ => handler_name(expr).into_iter().collect(),
    }
}

fn raised_exception_name(expr: &Expr, active_except: Option<&ExceptContext>) -> Option<String> {
    match expr {
        Expr::Call(node) => qualified_name(&node.func),
        Expr::Name(node) => {
            if let Some(context) = active_except {
                if context.binding.as_deref() == Some(node.id.as_str()) {
                    // A named re-raise preserves every exception type from a
                    // tuple handler: `except (A, B) as exc: raise exc`.
                    // The caller inserts a single string, so only collapse the
                    // binding when the handler has one exact type; tuple
                    // handlers are expanded by `raised_exception_names`.
                    if context.types.len() == 1 {
                        return context.types.first().cloned();
                    }
                }
            }
            Some(node.id.to_string())
        }
        Expr::Attribute(_) => qualified_name(expr),
        _ => qualified_name(expr),
    }
}

fn walk_expr(expr: &Expr, facts: &mut FunctionFacts) {
    match expr {
        Expr::Yield(node) => {
            facts.has_yield = true;
            if let Some(value) = &node.value {
                walk_expr(value, facts);
            }
        }
        Expr::YieldFrom(node) => {
            facts.has_yield = true;
            walk_expr(&node.value, facts);
        }
        Expr::Lambda(_) => {
            // Treat lambda as a nested callable scope for control-flow facts.
        }
        Expr::BoolOp(node) => {
            for value in &node.values {
                walk_expr(value, facts);
            }
        }
        Expr::NamedExpr(node) => {
            walk_expr(&node.target, facts);
            walk_expr(&node.value, facts);
        }
        Expr::BinOp(node) => {
            walk_expr(&node.left, facts);
            walk_expr(&node.right, facts);
        }
        Expr::UnaryOp(node) => walk_expr(&node.operand, facts),
        Expr::IfExp(node) => {
            walk_expr(&node.test, facts);
            walk_expr(&node.body, facts);
            walk_expr(&node.orelse, facts);
        }
        Expr::Dict(node) => {
            for key in node.keys.iter().flatten() {
                walk_expr(key, facts);
            }
            for value in &node.values {
                walk_expr(value, facts);
            }
        }
        Expr::Set(node) => {
            for value in &node.elts {
                walk_expr(value, facts);
            }
        }
        Expr::ListComp(node) => {
            walk_expr(&node.elt, facts);
            walk_comprehensions(&node.generators, facts);
        }
        Expr::SetComp(node) => {
            walk_expr(&node.elt, facts);
            walk_comprehensions(&node.generators, facts);
        }
        Expr::DictComp(node) => {
            walk_expr(&node.key, facts);
            walk_expr(&node.value, facts);
            walk_comprehensions(&node.generators, facts);
        }
        Expr::GeneratorExp(node) => {
            walk_expr(&node.elt, facts);
            walk_comprehensions(&node.generators, facts);
        }
        Expr::Await(node) => walk_expr(&node.value, facts),
        Expr::Compare(node) => {
            walk_expr(&node.left, facts);
            for value in &node.comparators {
                walk_expr(value, facts);
            }
        }
        Expr::Call(node) => {
            walk_expr(&node.func, facts);
            for arg in &node.args {
                walk_expr(arg, facts);
            }
            for keyword in &node.keywords {
                walk_expr(&keyword.value, facts);
            }
        }
        Expr::FormattedValue(node) => {
            walk_expr(&node.value, facts);
            if let Some(spec) = &node.format_spec {
                walk_expr(spec, facts);
            }
        }
        Expr::JoinedStr(node) => {
            for value in &node.values {
                walk_expr(value, facts);
            }
        }
        Expr::Attribute(node) => walk_expr(&node.value, facts),
        Expr::Subscript(node) => {
            walk_expr(&node.value, facts);
            walk_expr(&node.slice, facts);
        }
        Expr::Starred(node) => walk_expr(&node.value, facts),
        Expr::List(node) => {
            for value in &node.elts {
                walk_expr(value, facts);
            }
        }
        Expr::Tuple(node) => {
            for value in &node.elts {
                walk_expr(value, facts);
            }
        }
        Expr::Slice(node) => {
            if let Some(value) = &node.lower {
                walk_expr(value, facts);
            }
            if let Some(value) = &node.upper {
                walk_expr(value, facts);
            }
            if let Some(value) = &node.step {
                walk_expr(value, facts);
            }
        }
        Expr::Constant(_) | Expr::Name(_) => {}
    }
}

fn walk_comprehensions(items: &[ast::Comprehension], facts: &mut FunctionFacts) {
    for item in items {
        walk_expr(&item.target, facts);
        walk_expr(&item.iter, facts);
        for condition in &item.ifs {
            walk_expr(condition, facts);
        }
    }
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

fn text_size_to_usize(size: ast::text_size::TextSize) -> usize {
    u32::from(size) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_understands_multiline_class_and_annotation() {
        let source = r#"
class Child(
    BaseOne,
    BaseTwo,
):
    value: dict[
        str,
        list[int],
    ]
"#;
        let parsed = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::ClassDef(class) = &parsed.suite[0] else {
            panic!("expected class");
        };
        assert_eq!(class.name.as_str(), "Child");
        assert_eq!(class.bases.len(), 2);
        let Stmt::AnnAssign(field) = &class.body[0] else {
            panic!("expected annotated field");
        };
        assert_eq!(
            expression_text(source, &field.annotation),
            "dict[\n        str,\n        list[int],\n    ]"
        );
    }

    #[test]
    fn function_facts_ignore_nested_functions_and_classes() {
        let source = r#"
def outer(flag: bool):
    def nested():
        raise ValueError()
        yield 1
        return 1

    class Nested:
        def method(self):
            raise RuntimeError()

    if flag:
        raise LookupError()
    return
"#;
        let parsed = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::FunctionDef(function) = &parsed.suite[0] else {
            panic!("expected function");
        };
        let facts = function_facts(&function.body);
        assert!(facts.has_return);
        assert!(facts.has_bare_return);
        assert!(!facts.has_yield);
        assert!(facts.has_raise);
        assert_eq!(facts.raised_exceptions, vec!["LookupError"]);
    }

    #[test]
    fn decorator_qualified_name_unwraps_calls() {
        let source = "@typing.dataclass_transform()\ndef model(cls):\n    return cls\n";
        let parsed = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::FunctionDef(function) = &parsed.suite[0] else {
            panic!("expected function");
        };
        assert_eq!(
            qualified_name(&function.decorator_list[0]).as_deref(),
            Some("typing.dataclass_transform")
        );
    }

    #[test]
    fn named_reraise_uses_bound_exception_types() {
        let source = r#"
def f():
    try:
        work()
    except (ValueError, TypeError) as exc:
        raise exc
"#;
        let parsed = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::FunctionDef(function) = &parsed.suite[0] else {
            panic!("expected function");
        };
        let facts = function_facts(&function.body);
        assert_eq!(facts.raised_exceptions, vec!["TypeError", "ValueError"]);
    }

    #[test]
    fn unrelated_raise_inside_except_is_not_rewritten_to_handler_type() {
        let source = r#"
def f():
    try:
        work()
    except ValueError as exc:
        raise RuntimeError()
"#;
        let parsed = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::FunctionDef(function) = &parsed.suite[0] else {
            panic!("expected function");
        };
        let facts = function_facts(&function.body);
        assert_eq!(facts.raised_exceptions, vec!["RuntimeError"]);
    }

    #[test]
    fn bare_reraise_uses_all_handler_types() {
        let source = r#"
def f():
    try:
        work()
    except (LookupError, OSError):
        raise
"#;
        let parsed = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::FunctionDef(function) = &parsed.suite[0] else {
            panic!("expected function");
        };
        let facts = function_facts(&function.body);
        assert_eq!(facts.raised_exceptions, vec!["LookupError", "OSError"]);
    }
}
