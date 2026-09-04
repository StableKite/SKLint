//! Unified magic-number rule (SK901).
//!
//! The rule deliberately combines two complementary policies:
//! - WPS432-style broad checking of unnamed numeric constants in expressions,
//!   while allowing self-documenting/direct literal contexts such as assignments,
//!   defaults, walrus assignments, container literals and `Literal[...]`;
//! - Ruff PLR2004-style stricter checking of comparison operands, including
//!   interpreter-version exemptions.
//!
//! All ownership/context decisions are made from the RustPython AST.

use crate::config::EffectiveConfig;
use crate::diagnostic::{Diagnostic, Span};
use crate::python_ast::{qualified_name, source_text, PythonAst};
use rustpython_parser::ast::{self, Constant, Expr, Stmt, UnaryOp};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Default)]
struct Scope {
    sys_modules: HashSet<String>,
    sys_version_values: HashSet<String>,
    sys_implementation_values: HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExprContext {
    Normal,
    /// WPS direct, self-documenting contexts where a direct number is allowed.
    DirectLiteralSafe,
    /// A direct comparison operand. Ruff comparison logic owns the literal.
    ComparisonOperand,
    /// RHS of an explicitly named constant. Literal-building operations are
    /// self-documenting, but comparison operands inside the RHS remain magic.
    NamedConstantValue,
}

fn nested_expr_context(parent: ExprContext, default: ExprContext) -> ExprContext {
    if parent == ExprContext::NamedConstantValue {
        ExprContext::NamedConstantValue
    } else {
        default
    }
}

struct Visitor<'a> {
    path: String,
    source: &'a str,
    ast: &'a PythonAst,
    diagnostics: Vec<Diagnostic>,
}

pub fn run_magic_constant_rule(
    path: &Path,
    source: &str,
    config: &EffectiveConfig,
) -> Vec<Diagnostic> {
    if !config.is_enabled("SK901") {
        return Vec::new();
    }
    let display_path = path.display().to_string();
    let Ok(ast) = PythonAst::parse(source, &display_path) else {
        // Syntax ownership belongs to SKD002 / parser diagnostics. Avoid a
        // duplicate parser error from this independent rule.
        return Vec::new();
    };
    let mut visitor = Visitor {
        path: display_path,
        source,
        ast: &ast,
        diagnostics: Vec::new(),
    };
    let scope = scope_for_body(&Scope::default(), &ast.suite, &[]);
    visitor.visit_statements(&ast.suite, &scope, false);
    visitor.diagnostics
}

impl Visitor<'_> {
    fn visit_statements(&mut self, body: &[Stmt], scope: &Scope, main_exempt: bool) {
        if main_exempt {
            return;
        }
        for stmt in body {
            match stmt {
                Stmt::FunctionDef(node) => {
                    for decorator in &node.decorator_list {
                        self.visit_expr(decorator, scope, ExprContext::Normal);
                    }
                    for arg in &node.args.posonlyargs {
                        if let Some(default) = &arg.default {
                            self.visit_expr(default, scope, ExprContext::DirectLiteralSafe);
                        }
                        if let Some(annotation) = &arg.def.annotation {
                            self.visit_annotation(annotation, scope);
                        }
                    }
                    for arg in &node.args.args {
                        if let Some(default) = &arg.default {
                            self.visit_expr(default, scope, ExprContext::DirectLiteralSafe);
                        }
                        if let Some(annotation) = &arg.def.annotation {
                            self.visit_annotation(annotation, scope);
                        }
                    }
                    if let Some(arg) = &node.args.vararg {
                        if let Some(annotation) = &arg.annotation {
                            self.visit_annotation(annotation, scope);
                        }
                    }
                    for arg in &node.args.kwonlyargs {
                        if let Some(default) = &arg.default {
                            self.visit_expr(default, scope, ExprContext::DirectLiteralSafe);
                        }
                        if let Some(annotation) = &arg.def.annotation {
                            self.visit_annotation(annotation, scope);
                        }
                    }
                    if let Some(arg) = &node.args.kwarg {
                        if let Some(annotation) = &arg.annotation {
                            self.visit_annotation(annotation, scope);
                        }
                    }
                    if let Some(returns) = &node.returns {
                        self.visit_annotation(returns, scope);
                    }
                    let bound = function_bound_names(&node.args);
                    let child_scope = scope_for_body(scope, &node.body, &bound);
                    self.visit_statements(&node.body, &child_scope, false);
                }
                Stmt::AsyncFunctionDef(node) => {
                    for decorator in &node.decorator_list {
                        self.visit_expr(decorator, scope, ExprContext::Normal);
                    }
                    for arg in &node.args.posonlyargs {
                        if let Some(default) = &arg.default {
                            self.visit_expr(default, scope, ExprContext::DirectLiteralSafe);
                        }
                        if let Some(annotation) = &arg.def.annotation {
                            self.visit_annotation(annotation, scope);
                        }
                    }
                    for arg in &node.args.args {
                        if let Some(default) = &arg.default {
                            self.visit_expr(default, scope, ExprContext::DirectLiteralSafe);
                        }
                        if let Some(annotation) = &arg.def.annotation {
                            self.visit_annotation(annotation, scope);
                        }
                    }
                    if let Some(arg) = &node.args.vararg {
                        if let Some(annotation) = &arg.annotation {
                            self.visit_annotation(annotation, scope);
                        }
                    }
                    for arg in &node.args.kwonlyargs {
                        if let Some(default) = &arg.default {
                            self.visit_expr(default, scope, ExprContext::DirectLiteralSafe);
                        }
                        if let Some(annotation) = &arg.def.annotation {
                            self.visit_annotation(annotation, scope);
                        }
                    }
                    if let Some(arg) = &node.args.kwarg {
                        if let Some(annotation) = &arg.annotation {
                            self.visit_annotation(annotation, scope);
                        }
                    }
                    if let Some(returns) = &node.returns {
                        self.visit_annotation(returns, scope);
                    }
                    let bound = function_bound_names(&node.args);
                    let child_scope = scope_for_body(scope, &node.body, &bound);
                    self.visit_statements(&node.body, &child_scope, false);
                }
                Stmt::ClassDef(node) => {
                    for base in &node.bases {
                        self.visit_expr(base, scope, ExprContext::Normal);
                    }
                    for keyword in &node.keywords {
                        self.visit_expr(&keyword.value, scope, ExprContext::Normal);
                    }
                    for decorator in &node.decorator_list {
                        self.visit_expr(decorator, scope, ExprContext::Normal);
                    }
                    let child_scope = scope_for_body(scope, &node.body, &[]);
                    self.visit_statements(&node.body, &child_scope, false);
                }
                Stmt::Return(node) => {
                    if let Some(value) = &node.value {
                        self.visit_expr(value, scope, ExprContext::Normal);
                    }
                }
                Stmt::Delete(node) => {
                    for target in &node.targets {
                        self.visit_expr(target, scope, ExprContext::Normal);
                    }
                }
                Stmt::Assign(node) => {
                    for target in &node.targets {
                        self.visit_expr(target, scope, ExprContext::Normal);
                    }
                    let context = if node.targets.iter().all(is_named_constant_target) {
                        ExprContext::NamedConstantValue
                    } else {
                        ExprContext::DirectLiteralSafe
                    };
                    self.visit_expr(&node.value, scope, context);
                }
                Stmt::TypeAlias(node) => {
                    self.visit_expr(&node.name, scope, ExprContext::Normal);
                    self.visit_expr(&node.value, scope, ExprContext::Normal);
                }
                Stmt::AugAssign(node) => {
                    self.visit_expr(&node.target, scope, ExprContext::Normal);
                    self.visit_expr(&node.value, scope, ExprContext::Normal);
                }
                Stmt::AnnAssign(node) => {
                    self.visit_expr(&node.target, scope, ExprContext::Normal);
                    self.visit_annotation(&node.annotation, scope);
                    if let Some(value) = &node.value {
                        let context = if is_named_constant_target(&node.target)
                            || is_final_annotation(&node.annotation)
                        {
                            ExprContext::NamedConstantValue
                        } else {
                            ExprContext::DirectLiteralSafe
                        };
                        self.visit_expr(value, scope, context);
                    }
                }
                Stmt::For(node) => {
                    self.visit_expr(&node.target, scope, ExprContext::Normal);
                    self.visit_expr(&node.iter, scope, ExprContext::Normal);
                    self.visit_statements(&node.body, scope, false);
                    self.visit_statements(&node.orelse, scope, false);
                }
                Stmt::AsyncFor(node) => {
                    self.visit_expr(&node.target, scope, ExprContext::Normal);
                    self.visit_expr(&node.iter, scope, ExprContext::Normal);
                    self.visit_statements(&node.body, scope, false);
                    self.visit_statements(&node.orelse, scope, false);
                }
                Stmt::While(node) => {
                    self.visit_expr(&node.test, scope, ExprContext::Normal);
                    self.visit_statements(&node.body, scope, false);
                    self.visit_statements(&node.orelse, scope, false);
                }
                Stmt::If(node) => {
                    self.visit_expr(&node.test, scope, ExprContext::Normal);
                    let exact_main = is_exact_main_guard(&node.test);
                    self.visit_statements(&node.body, scope, exact_main);
                    self.visit_statements(&node.orelse, scope, false);
                }
                Stmt::With(node) => {
                    for item in &node.items {
                        self.visit_expr(&item.context_expr, scope, ExprContext::Normal);
                        if let Some(vars) = &item.optional_vars {
                            self.visit_expr(vars, scope, ExprContext::Normal);
                        }
                    }
                    self.visit_statements(&node.body, scope, false);
                }
                Stmt::AsyncWith(node) => {
                    for item in &node.items {
                        self.visit_expr(&item.context_expr, scope, ExprContext::Normal);
                        if let Some(vars) = &item.optional_vars {
                            self.visit_expr(vars, scope, ExprContext::Normal);
                        }
                    }
                    self.visit_statements(&node.body, scope, false);
                }
                Stmt::Match(node) => {
                    self.visit_expr(&node.subject, scope, ExprContext::Normal);
                    for case in &node.cases {
                        if let Some(guard) = &case.guard {
                            self.visit_expr(guard, scope, ExprContext::Normal);
                        }
                        self.visit_statements(&case.body, scope, false);
                    }
                }
                Stmt::Raise(node) => {
                    if let Some(exc) = &node.exc {
                        self.visit_expr(exc, scope, ExprContext::Normal);
                    }
                    if let Some(cause) = &node.cause {
                        self.visit_expr(cause, scope, ExprContext::Normal);
                    }
                }
                Stmt::Try(node) => {
                    self.visit_statements(&node.body, scope, false);
                    for handler in &node.handlers {
                        let ast::ExceptHandler::ExceptHandler(handler) = handler;
                        if let Some(type_expr) = &handler.type_ {
                            self.visit_expr(type_expr, scope, ExprContext::Normal);
                        }
                        self.visit_statements(&handler.body, scope, false);
                    }
                    self.visit_statements(&node.orelse, scope, false);
                    self.visit_statements(&node.finalbody, scope, false);
                }
                Stmt::TryStar(node) => {
                    self.visit_statements(&node.body, scope, false);
                    for handler in &node.handlers {
                        let ast::ExceptHandler::ExceptHandler(handler) = handler;
                        if let Some(type_expr) = &handler.type_ {
                            self.visit_expr(type_expr, scope, ExprContext::Normal);
                        }
                        self.visit_statements(&handler.body, scope, false);
                    }
                    self.visit_statements(&node.orelse, scope, false);
                    self.visit_statements(&node.finalbody, scope, false);
                }
                Stmt::Assert(node) => {
                    self.visit_expr(&node.test, scope, ExprContext::Normal);
                    if let Some(msg) = &node.msg {
                        self.visit_expr(msg, scope, ExprContext::Normal);
                    }
                }
                Stmt::Expr(node) => self.visit_expr(&node.value, scope, ExprContext::Normal),
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

    fn visit_annotation(&mut self, expr: &Expr, scope: &Scope) {
        if is_literal_annotation(expr) {
            if let Expr::Subscript(node) = expr {
                self.visit_expr(&node.value, scope, ExprContext::Normal);
                self.visit_expr(&node.slice, scope, ExprContext::DirectLiteralSafe);
            }
        } else {
            self.visit_expr(expr, scope, ExprContext::Normal);
        }
    }

    fn visit_expr(&mut self, expr: &Expr, scope: &Scope, context: ExprContext) {
        if let Some(text) = numeric_literal_text(self.source, expr) {
            match context {
                ExprContext::DirectLiteralSafe
                | ExprContext::ComparisonOperand
                | ExprContext::NamedConstantValue => return,
                ExprContext::Normal => {
                    if !wps_number_allowed(&text) {
                        self.report(expr, &text);
                    }
                    return;
                }
            }
        }

        match expr {
            Expr::BoolOp(node) => {
                for value in &node.values {
                    self.visit_expr(
                        value,
                        scope,
                        nested_expr_context(context, ExprContext::Normal),
                    );
                }
            }
            Expr::NamedExpr(node) => {
                self.visit_expr(
                    &node.target,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
                self.visit_expr(
                    &node.value,
                    scope,
                    nested_expr_context(context, ExprContext::DirectLiteralSafe),
                );
            }
            Expr::BinOp(node) => {
                self.visit_expr(
                    &node.left,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
                self.visit_expr(
                    &node.right,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
            }
            Expr::UnaryOp(node) => self.visit_expr(
                &node.operand,
                scope,
                nested_expr_context(context, ExprContext::Normal),
            ),
            Expr::Lambda(node) => {
                for arg in &node.args.posonlyargs {
                    if let Some(default) = &arg.default {
                        self.visit_expr(
                            default,
                            scope,
                            nested_expr_context(context, ExprContext::DirectLiteralSafe),
                        );
                    }
                }
                for arg in &node.args.args {
                    if let Some(default) = &arg.default {
                        self.visit_expr(
                            default,
                            scope,
                            nested_expr_context(context, ExprContext::DirectLiteralSafe),
                        );
                    }
                }
                for arg in &node.args.kwonlyargs {
                    if let Some(default) = &arg.default {
                        self.visit_expr(
                            default,
                            scope,
                            nested_expr_context(context, ExprContext::DirectLiteralSafe),
                        );
                    }
                }
                self.visit_expr(
                    &node.body,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
            }
            Expr::IfExp(node) => {
                self.visit_expr(
                    &node.test,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
                self.visit_expr(
                    &node.body,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
                self.visit_expr(
                    &node.orelse,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
            }
            Expr::Dict(node) => {
                for key in node.keys.iter().flatten() {
                    self.visit_expr(
                        key,
                        scope,
                        nested_expr_context(context, ExprContext::DirectLiteralSafe),
                    );
                }
                for value in &node.values {
                    self.visit_expr(
                        value,
                        scope,
                        nested_expr_context(context, ExprContext::DirectLiteralSafe),
                    );
                }
            }
            Expr::Set(node) => {
                for value in &node.elts {
                    self.visit_expr(
                        value,
                        scope,
                        nested_expr_context(context, ExprContext::DirectLiteralSafe),
                    );
                }
            }
            Expr::List(node) => {
                for value in &node.elts {
                    self.visit_expr(
                        value,
                        scope,
                        nested_expr_context(context, ExprContext::DirectLiteralSafe),
                    );
                }
            }
            Expr::Tuple(node) => {
                for value in &node.elts {
                    self.visit_expr(
                        value,
                        scope,
                        nested_expr_context(context, ExprContext::DirectLiteralSafe),
                    );
                }
            }
            Expr::ListComp(node) => {
                self.visit_expr(
                    &node.elt,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
                self.visit_comprehensions(&node.generators, scope);
            }
            Expr::SetComp(node) => {
                self.visit_expr(
                    &node.elt,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
                self.visit_comprehensions(&node.generators, scope);
            }
            Expr::DictComp(node) => {
                self.visit_expr(
                    &node.key,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
                self.visit_expr(
                    &node.value,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
                self.visit_comprehensions(&node.generators, scope);
            }
            Expr::GeneratorExp(node) => {
                self.visit_expr(
                    &node.elt,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
                self.visit_comprehensions(&node.generators, scope);
            }
            Expr::Await(node) => self.visit_expr(
                &node.value,
                scope,
                nested_expr_context(context, ExprContext::Normal),
            ),
            Expr::Yield(node) => {
                if let Some(value) = &node.value {
                    self.visit_expr(
                        value,
                        scope,
                        nested_expr_context(context, ExprContext::Normal),
                    );
                }
            }
            Expr::YieldFrom(node) => self.visit_expr(
                &node.value,
                scope,
                nested_expr_context(context, ExprContext::Normal),
            ),
            Expr::Compare(node) => self.visit_comparison(&node.left, &node.comparators, scope),
            Expr::Call(node) => {
                self.visit_expr(
                    &node.func,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
                for arg in &node.args {
                    self.visit_expr(
                        arg,
                        scope,
                        nested_expr_context(context, ExprContext::Normal),
                    );
                }
                for keyword in &node.keywords {
                    self.visit_expr(
                        &keyword.value,
                        scope,
                        nested_expr_context(context, ExprContext::Normal),
                    );
                }
            }
            Expr::FormattedValue(node) => {
                self.visit_expr(
                    &node.value,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
                if let Some(spec) = &node.format_spec {
                    self.visit_expr(
                        spec,
                        scope,
                        nested_expr_context(context, ExprContext::Normal),
                    );
                }
            }
            Expr::JoinedStr(node) => {
                for value in &node.values {
                    self.visit_expr(
                        value,
                        scope,
                        nested_expr_context(context, ExprContext::Normal),
                    );
                }
            }
            Expr::Attribute(node) => self.visit_expr(
                &node.value,
                scope,
                nested_expr_context(context, ExprContext::Normal),
            ),
            Expr::Subscript(node) => {
                self.visit_expr(
                    &node.value,
                    scope,
                    nested_expr_context(context, ExprContext::Normal),
                );
                let slice_context = if is_literal_annotation(expr) {
                    ExprContext::DirectLiteralSafe
                } else {
                    ExprContext::Normal
                };
                self.visit_expr(&node.slice, scope, slice_context);
            }
            Expr::Starred(node) => self.visit_expr(
                &node.value,
                scope,
                nested_expr_context(context, ExprContext::Normal),
            ),
            Expr::Slice(node) => {
                if let Some(value) = &node.lower {
                    self.visit_expr(
                        value,
                        scope,
                        nested_expr_context(context, ExprContext::Normal),
                    );
                }
                if let Some(value) = &node.upper {
                    self.visit_expr(
                        value,
                        scope,
                        nested_expr_context(context, ExprContext::Normal),
                    );
                }
                if let Some(value) = &node.step {
                    self.visit_expr(
                        value,
                        scope,
                        nested_expr_context(context, ExprContext::Normal),
                    );
                }
            }
            Expr::Constant(_) | Expr::Name(_) => {}
        }
    }

    fn visit_comparison(&mut self, left: &Expr, comparators: &[Expr], scope: &Scope) {
        let operands = std::iter::once(left).chain(comparators).collect::<Vec<_>>();
        if operands
            .windows(2)
            .any(|pair| is_any_literal(pair[0]) && is_any_literal(pair[1]))
        {
            return;
        }

        for (index, expr) in operands.iter().enumerate() {
            if let Some(text) = numeric_literal_text(self.source, expr) {
                let previous_is_version = index
                    .checked_sub(1)
                    .is_some_and(|previous| is_sys_version_comparand(operands[previous], scope));
                let next_is_version = operands
                    .get(index + 1)
                    .is_some_and(|next| is_sys_version_comparand(next, scope));
                if !ruff_comparison_number_allowed(&text)
                    && !previous_is_version
                    && !next_is_version
                {
                    self.report(expr, &text);
                }
            } else {
                self.visit_expr(expr, scope, ExprContext::ComparisonOperand);
            }
        }
    }

    fn visit_comprehensions(&mut self, items: &[ast::Comprehension], scope: &Scope) {
        for item in items {
            self.visit_expr(&item.target, scope, ExprContext::Normal);
            self.visit_expr(&item.iter, scope, ExprContext::Normal);
            for condition in &item.ifs {
                self.visit_expr(condition, scope, ExprContext::Normal);
            }
        }
    }

    fn report(&mut self, expr: &Expr, text: &str) {
        let start = self.ast.location_of(expr);
        let end = self.ast.end_location_of(expr);
        self.diagnostics.push(Diagnostic::new(
            "SK901",
            format!("Magic numeric value `{text}` should be replaced with a named constant"),
            self.path.clone(),
            Span::new(
                start.line,
                start.column,
                end.line,
                end.column.max(start.column + 1),
            ),
            "warning",
        ));
    }
}

fn is_exact_main_guard(expr: &Expr) -> bool {
    let Expr::Compare(node) = expr else {
        return false;
    };
    if node.ops.len() != 1 || node.comparators.len() != 1 || node.ops[0] != ast::CmpOp::Eq {
        return false;
    }
    let Expr::Name(left) = node.left.as_ref() else {
        return false;
    };
    if left.id.as_str() != "__name__" {
        return false;
    }
    matches!(
        &node.comparators[0],
        Expr::Constant(constant) if matches!(&constant.value, Constant::Str(value) if value.as_str() == "__main__")
    )
}

fn is_literal_annotation(expr: &Expr) -> bool {
    let Expr::Subscript(node) = expr else {
        return false;
    };
    qualified_name(&node.value).is_some_and(|name| {
        matches!(
            name.as_str(),
            "Literal" | "typing.Literal" | "typing_extensions.Literal"
        )
    })
}

fn is_any_literal(expr: &Expr) -> bool {
    match expr {
        Expr::Constant(_) => true,
        Expr::UnaryOp(node)
            if matches!(node.op, UnaryOp::UAdd | UnaryOp::USub | UnaryOp::Invert) =>
        {
            matches!(node.operand.as_ref(), Expr::Constant(_))
        }
        _ => false,
    }
}

fn numeric_literal_text(source: &str, expr: &Expr) -> Option<String> {
    let text = source_text(source, expr).trim();
    match expr {
        Expr::Constant(_) if numeric_value(text).is_some() => Some(text.to_string()),
        Expr::UnaryOp(node)
            if matches!(node.op, UnaryOp::UAdd | UnaryOp::USub | UnaryOp::Invert)
                && matches!(node.operand.as_ref(), Expr::Constant(_))
                && numeric_value(source_text(source, node.operand.as_ref()).trim()).is_some() =>
        {
            Some(text.to_string())
        }
        _ => None,
    }
}

fn numeric_value(text: &str) -> Option<f64> {
    let mut text = text.trim().replace('_', "");
    while let Some(first) = text.chars().next() {
        if matches!(first, '+' | '-' | '~') {
            text.remove(0);
        } else {
            break;
        }
    }
    let text = text.trim();
    let text = text
        .strip_suffix('j')
        .or_else(|| text.strip_suffix('J'))
        .unwrap_or(text);
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u128::from_str_radix(hex, 16).ok().map(|value| value as f64);
    }
    if let Some(oct) = text.strip_prefix("0o").or_else(|| text.strip_prefix("0O")) {
        return u128::from_str_radix(oct, 8).ok().map(|value| value as f64);
    }
    if let Some(bin) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
        return u128::from_str_radix(bin, 2).ok().map(|value| value as f64);
    }
    text.parse::<f64>().ok()
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum WpsNumericLiteral {
    Integer(f64),
    Real(f64),
    Imaginary(f64),
}

fn wps_numeric_literal(text: &str) -> Option<WpsNumericLiteral> {
    let mut text = text.trim().replace('_', "");
    while let Some(first) = text.chars().next() {
        if matches!(first, '+' | '-' | '~') {
            text.remove(0);
        } else {
            break;
        }
    }
    let text = text.trim();
    if let Some(imaginary) = text.strip_suffix('j').or_else(|| text.strip_suffix('J')) {
        return imaginary
            .parse::<f64>()
            .ok()
            .map(WpsNumericLiteral::Imaginary);
    }
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u128::from_str_radix(hex, 16)
            .ok()
            .map(|value| WpsNumericLiteral::Integer(value as f64));
    }
    if let Some(oct) = text.strip_prefix("0o").or_else(|| text.strip_prefix("0O")) {
        return u128::from_str_radix(oct, 8)
            .ok()
            .map(|value| WpsNumericLiteral::Integer(value as f64));
    }
    if let Some(bin) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
        return u128::from_str_radix(bin, 2)
            .ok()
            .map(|value| WpsNumericLiteral::Integer(value as f64));
    }
    if !text.contains(['.', 'e', 'E']) {
        if let Ok(value) = text.parse::<u128>() {
            return Some(WpsNumericLiteral::Integer(value as f64));
        }
    }
    text.parse::<f64>().ok().map(WpsNumericLiteral::Real)
}

fn wps_number_allowed(text: &str) -> bool {
    let Some(number) = wps_numeric_literal(text) else {
        return true;
    };

    // WPS432 treats direct integer constants <= 10 as non-magic, but that
    // special case is deliberately type-sensitive: `10` is allowed while
    // `10.0` is not. Unary signs are AST parents in CPython, so the underlying
    // literal magnitude is what WPS observes; mirror that behavior here.
    if matches!(number, WpsNumericLiteral::Integer(value) if value <= 10.0) {
        return true;
    }

    // `MAGIC_NUMBERS_WHITELIST` in current WPS. Python numeric equality makes
    // the real-valued entries work across int/float spellings (e.g. 100.0),
    // while the complex whitelist contains only 1j (and 0j equals numeric 0).
    match number {
        WpsNumericLiteral::Integer(value) | WpsNumericLiteral::Real(value) => {
            [0.0, 0.1, 0.5, 1.0, 24.0, 60.0, 100.0, 1000.0, 1024.0].contains(&value)
        }
        WpsNumericLiteral::Imaginary(value) => value == 0.0 || value == 1.0,
    }
}

fn ruff_comparison_number_allowed(text: &str) -> bool {
    match wps_numeric_literal(text) {
        Some(WpsNumericLiteral::Integer(value) | WpsNumericLiteral::Real(value)) => {
            value == 0.0 || value == 1.0
        }
        Some(WpsNumericLiteral::Imaginary(_)) => false,
        None => true,
    }
}

fn is_sys_version_comparand(expr: &Expr, scope: &Scope) -> bool {
    let expr = unwrap_subscript(expr);
    if let Expr::Name(name) = expr {
        return scope.sys_version_values.contains(name.id.as_str());
    }
    let Some(name) = qualified_name(expr) else {
        return false;
    };
    if scope
        .sys_version_values
        .iter()
        .any(|alias| name == *alias || name.starts_with(&format!("{alias}.")))
    {
        return true;
    }
    for alias in &scope.sys_modules {
        if name == format!("{alias}.version")
            || name.starts_with(&format!("{alias}.version."))
            || name == format!("{alias}.version_info")
            || name.starts_with(&format!("{alias}.version_info."))
            || name == format!("{alias}.implementation.version")
            || name.starts_with(&format!("{alias}.implementation.version."))
        {
            return true;
        }
    }
    for alias in &scope.sys_implementation_values {
        if name == format!("{alias}.version") || name.starts_with(&format!("{alias}.version.")) {
            return true;
        }
    }
    false
}

fn unwrap_subscript(mut expr: &Expr) -> &Expr {
    while let Expr::Subscript(node) = expr {
        expr = &node.value;
    }
    expr
}

fn is_named_constant_target(expr: &Expr) -> bool {
    matches!(expr, Expr::Name(name) if {
        let value = name.id.as_str();
        !value.is_empty()
            && value.chars().any(|ch| ch.is_ascii_alphabetic())
            && value.chars().all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
    })
}

fn is_final_annotation(expr: &Expr) -> bool {
    qualified_name(expr).is_some_and(|name| name == "Final" || name.ends_with(".Final"))
}

fn scope_for_body(parent: &Scope, body: &[Stmt], initial_bound: &[String]) -> Scope {
    let mut scope = parent.clone();
    let mut shadowed = initial_bound.iter().cloned().collect::<HashSet<_>>();
    collect_non_import_bindings(body, &mut shadowed);
    for name in &shadowed {
        scope.sys_modules.remove(name);
        scope.sys_version_values.remove(name);
        scope.sys_implementation_values.remove(name);
    }
    collect_sys_imports(body, &shadowed, &mut scope);
    scope
}

fn collect_sys_imports(body: &[Stmt], shadowed: &HashSet<String>, scope: &mut Scope) {
    for stmt in body {
        match stmt {
            Stmt::Import(node) => {
                for alias in &node.names {
                    if alias.name.as_str() != "sys" {
                        continue;
                    }
                    let binding = alias
                        .asname
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "sys".to_string());
                    if !shadowed.contains(&binding) {
                        scope.sys_modules.insert(binding);
                    }
                }
            }
            Stmt::ImportFrom(node) if node.module.as_ref().is_some_and(|m| m.as_str() == "sys") => {
                for alias in &node.names {
                    let binding = alias
                        .asname
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| alias.name.to_string());
                    if shadowed.contains(&binding) {
                        continue;
                    }
                    match alias.name.as_str() {
                        "version" | "version_info" => {
                            scope.sys_version_values.insert(binding);
                        }
                        "implementation" => {
                            scope.sys_implementation_values.insert(binding);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_non_import_bindings(body: &[Stmt], out: &mut HashSet<String>) {
    for stmt in body {
        match stmt {
            Stmt::Assign(node) => {
                for target in &node.targets {
                    collect_target_names(target, out);
                }
            }
            Stmt::AnnAssign(node) => collect_target_names(&node.target, out),
            Stmt::AugAssign(node) => collect_target_names(&node.target, out),
            Stmt::For(node) => {
                collect_target_names(&node.target, out);
                collect_non_import_bindings(&node.body, out);
                collect_non_import_bindings(&node.orelse, out);
            }
            Stmt::AsyncFor(node) => {
                collect_target_names(&node.target, out);
                collect_non_import_bindings(&node.body, out);
                collect_non_import_bindings(&node.orelse, out);
            }
            Stmt::With(node) => {
                for item in &node.items {
                    if let Some(target) = &item.optional_vars {
                        collect_target_names(target, out);
                    }
                }
            }
            Stmt::AsyncWith(node) => {
                for item in &node.items {
                    if let Some(target) = &item.optional_vars {
                        collect_target_names(target, out);
                    }
                }
            }
            Stmt::FunctionDef(node) => {
                out.insert(node.name.to_string());
            }
            Stmt::AsyncFunctionDef(node) => {
                out.insert(node.name.to_string());
            }
            Stmt::ClassDef(node) => {
                out.insert(node.name.to_string());
            }
            _ => {}
        }
    }
}

fn collect_target_names(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Name(node) => {
            out.insert(node.id.to_string());
        }
        Expr::Tuple(node) => {
            for element in &node.elts {
                collect_target_names(element, out);
            }
        }
        Expr::List(node) => {
            for element in &node.elts {
                collect_target_names(element, out);
            }
        }
        Expr::Starred(node) => collect_target_names(&node.value, out),
        _ => {}
    }
}

fn function_bound_names(args: &ast::Arguments) -> Vec<String> {
    let mut names = Vec::new();
    names.extend(args.posonlyargs.iter().map(|arg| arg.def.arg.to_string()));
    names.extend(args.args.iter().map(|arg| arg.def.arg.to_string()));
    if let Some(arg) = &args.vararg {
        names.push(arg.arg.to_string());
    }
    names.extend(args.kwonlyargs.iter().map(|arg| arg.def.arg.to_string()));
    if let Some(arg) = &args.kwarg {
        names.push(arg.arg.to_string());
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EffectiveConfig, FileInlineConfig, PyProjectConfig, VscodeConfig};

    fn strict_config() -> EffectiveConfig {
        EffectiveConfig::resolve(
            &VscodeConfig::default(),
            &PyProjectConfig::default(),
            &FileInlineConfig {
                strict: Some(true),
                select: vec!["SK901".into()],
                ..FileInlineConfig::default()
            },
        )
    }

    fn diagnostics(source: &str) -> Vec<Diagnostic> {
        run_magic_constant_rule(Path::new("example.py"), source, &strict_config())
    }

    #[test]
    fn direct_assignment_defaults_containers_and_literal_are_allowed() {
        let source = r#"from typing import Literal
x = 999

def f(value=999):
    data = [999, {999: (999,)}]
    typed: Literal[999]
"#;
        assert!(diagnostics(source).is_empty());
    }

    #[test]
    fn named_constant_expression_is_self_documenting() {
        let found = diagnostics("TCGETS2: Final = getattr(termios, \"TCGETS2\", 0x802C542A)\n");
        assert!(found.is_empty());
        let found = diagnostics("MASK = 1 << 31\n");
        assert!(found.is_empty());
    }

    #[test]
    fn named_constant_still_checks_comparison_operands() {
        let found = diagnostics("SPECIAL = get_value() == 999\n");
        assert_eq!(found.iter().filter(|diag| diag.code == "SK901").count(), 1);

        let found = diagnostics("IS_SPECIAL: Final = get_value() == 998\n");
        assert_eq!(found.iter().filter(|diag| diag.code == "SK901").count(), 1);
    }

    #[test]
    fn named_constant_literal_building_remains_allowed() {
        assert!(diagnostics("MASK = 1 << 31\n").is_empty());
        assert!(
            diagnostics("TIOCMGET: Final = getattr(termios, \"TIOCMGET\", 0x5415)\n").is_empty()
        );
    }

    #[test]
    fn arithmetic_magic_number_is_reported() {
        let found = diagnostics("result = value * 999\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].code, "SK901");
    }

    #[test]
    fn comparison_uses_ruff_stricter_zero_one_policy() {
        assert_eq!(diagnostics("if value == 2:\n    pass\n").len(), 1);
        assert!(diagnostics("if value == 0:\n    pass\n").is_empty());
        assert!(diagnostics("if value == 1:\n    pass\n").is_empty());
    }

    #[test]
    fn exact_main_guard_body_is_fully_exempt() {
        let source = "if __name__ == \"__main__\":\n    value = foo * 999\n    if value == 123:\n        print(456)\n";
        assert!(diagnostics(source).is_empty());
        assert_eq!(
            diagnostics("if __name__ != \"__main__\":\n    value = foo * 999\n").len(),
            1
        );
    }

    #[test]
    fn imported_sys_version_comparisons_are_exempt() {
        assert!(diagnostics("import sys\nif sys.version_info >= (3, 13):\n    pass\n").is_empty());
        assert!(diagnostics(
            "import sys as runtime\nif runtime.version_info[0] >= 13:\n    pass\n"
        )
        .is_empty());
        assert!(
            diagnostics("from sys import version_info as vi\nif vi.major >= 13:\n    pass\n")
                .is_empty()
        );
    }

    #[test]
    fn arbitrary_or_shadowed_sys_is_not_exempt() {
        assert_eq!(
            diagnostics("if sys.version_info >= 13:\n    pass\n").len(),
            1
        );
        let source = "import sys\ndef f(sys):\n    if sys.version_info >= 13:\n        pass\n";
        assert_eq!(diagnostics(source).len(), 1);
    }

    #[test]
    fn literal_to_literal_pair_skips_comparison_like_ruff() {
        assert!(diagnostics("if 13 < 14 < value:\n    pass\n").is_empty());
    }

    #[test]
    fn wps_keyword_arguments_are_checked_but_direct_walrus_values_are_allowed() {
        assert_eq!(diagnostics("print(end=999)\n").len(), 1);
        assert!(diagnostics("if (value := 999):\n    pass\n").is_empty());
        assert_eq!(
            diagnostics("if (value := other * 999):\n    pass\n").len(),
            1
        );
    }

    #[test]
    fn wps_small_integer_exception_does_not_apply_to_integer_valued_floats() {
        assert!(diagnostics("result = value + 10\n").is_empty());
        assert_eq!(diagnostics("result = value + 10.0\n").len(), 1);
        assert!(diagnostics("result = value + 100.0\n").is_empty());
        assert_eq!(diagnostics("result = value + 8.3\n").len(), 1);
    }

    #[test]
    fn wps_complex_whitelist_only_allows_zero_and_one_imaginary() {
        assert!(diagnostics("result = value + 1j\n").is_empty());
        assert!(diagnostics("result = value + 0j\n").is_empty());
        assert_eq!(diagnostics("result = value + 2j\n").len(), 1);
    }

    #[test]
    fn ruff_comparisons_treat_complex_zero_and_one_as_magic() {
        assert_eq!(diagnostics("if value == 1j:\n    pass\n").len(), 1);
        assert_eq!(diagnostics("if value == 0j:\n    pass\n").len(), 1);
        assert!(diagnostics("if value == 1.0:\n    pass\n").is_empty());
    }
}
