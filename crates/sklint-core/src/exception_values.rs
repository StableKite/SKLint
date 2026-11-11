//! Resolve annotated exception values without treating attribute spellings as types.
//!
//! This deliberately small flow analysis trusts declared builtin exception types,
//! proven property bindings, Python's class MRO, and explicit None guards. Unknown
//! bindings and unsupported control flow retain the ordinary Raises diagnostics.

use crate::dataclass_model::DataclassModel;
use crate::python_ast::{qualified_name, string_constant};
use rustpython_ast::Visitor;
use rustpython_parser::ast::{self, Expr, Ranged, Stmt};
use rustpython_parser::Parse;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExceptionType {
    pub names: Vec<String>,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Binding {
    Symbol(String),
    Class,
    Unknown,
}

#[derive(Clone, Default)]
pub(crate) struct Scope {
    bindings: HashMap<String, Vec<Binding>>,
    mutated: HashSet<String>,
    star: bool,
    conditional: bool,
}

impl Scope {
    fn outer_args(&mut self, args: &ast::Arguments) {
        for arg in args
            .posonlyargs
            .iter()
            .chain(&args.args)
            .chain(&args.kwonlyargs)
        {
            if let Some(expr) = arg.default.as_deref() {
                self.visit_expr(expr.clone());
            }
            if let Some(expr) = arg.def.annotation.as_deref() {
                self.visit_expr(expr.clone());
            }
        }
    }
    fn define(&mut self, name: &str, binding: Binding) {
        self.bindings
            .entry(name.to_string())
            .or_default()
            .push(if self.conditional {
                Binding::Unknown
            } else {
                binding
            });
    }

    fn proven(&self, name: &str) -> Option<&Binding> {
        if self.star || self.mutated.contains(name) {
            return None;
        }
        let bindings = self.bindings.get(name)?;
        (bindings.len() == 1).then(|| &bindings[0])
    }

    fn symbol(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Name(name) => {
                if let Some(Binding::Symbol(symbol)) = self.proven(name.id.as_str()) {
                    return Some(symbol.clone());
                }
                if !self.star && !self.bindings.contains_key(name.id.as_str()) {
                    return Some(format!("builtins.{}", name.id));
                }
                None
            }
            Expr::Attribute(attr) => Some(format!("{}.{}", self.symbol(&attr.value)?, attr.attr)),
            _ => None,
        }
    }
}

impl Visitor for Scope {
    fn visit_stmt_if(&mut self, node: ast::StmtIf) {
        self.visit_expr(*node.test);
        let previous = self.conditional;
        self.conditional = true;
        for stmt in node.body.into_iter().chain(node.orelse) {
            self.visit_stmt(stmt);
        }
        self.conditional = previous;
    }
    fn visit_stmt_function_def(&mut self, node: ast::StmtFunctionDef) {
        self.define(node.name.as_str(), Binding::Unknown);
        self.outer_args(&node.args);
        if let Some(expr) = node.returns {
            self.visit_expr(*expr);
        }
        for expr in node.decorator_list {
            self.visit_expr(expr);
        }
    }
    fn visit_stmt_async_function_def(&mut self, node: ast::StmtAsyncFunctionDef) {
        self.define(node.name.as_str(), Binding::Unknown);
        self.outer_args(&node.args);
        if let Some(expr) = node.returns {
            self.visit_expr(*expr);
        }
        for expr in node.decorator_list {
            self.visit_expr(expr);
        }
    }
    fn visit_stmt_class_def(&mut self, node: ast::StmtClassDef) {
        self.define(node.name.as_str(), Binding::Class);
        for keyword in node.keywords {
            self.visit_expr(keyword.value);
        }
        for expr in node.bases.into_iter().chain(node.decorator_list) {
            self.visit_expr(expr);
        }
    }
    fn visit_stmt_import(&mut self, node: ast::StmtImport) {
        for alias in node.names {
            let target = alias.name.to_string();
            let local = alias
                .asname
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| target.split('.').next().unwrap_or(&target).to_string());
            let symbol = if alias.asname.is_some() {
                target
            } else {
                local.clone()
            };
            self.define(&local, Binding::Symbol(symbol));
        }
    }
    fn visit_stmt_import_from(&mut self, node: ast::StmtImportFrom) {
        for alias in node.names {
            if alias.name.as_str() == "*" {
                self.star = true;
                continue;
            }
            let local = alias
                .asname
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| alias.name.to_string());
            let level = node
                .level
                .as_ref()
                .map(|level| level.to_usize())
                .unwrap_or(0);
            let module = node
                .module
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default();
            self.define(
                &local,
                Binding::Symbol(format!("{}{module}.{}", ".".repeat(level), alias.name)),
            );
        }
    }
    fn visit_expr_name(&mut self, node: ast::ExprName) {
        if matches!(node.ctx, ast::ExprContext::Store | ast::ExprContext::Del) {
            self.define(node.id.as_str(), Binding::Unknown);
        }
    }
    fn visit_expr_attribute(&mut self, node: ast::ExprAttribute) {
        if matches!(node.ctx, ast::ExprContext::Store | ast::ExprContext::Del) {
            if let Some(root) = qualified_name(&node.value)
                .and_then(|name| name.split('.').next().map(str::to_string))
            {
                self.mutated.insert(root);
            }
        }
        self.generic_visit_expr_attribute(node);
    }
    fn visit_expr_lambda(&mut self, _node: ast::ExprLambda) {}
}

pub(crate) fn annotation_scope(body: &[Stmt]) -> Scope {
    let mut scope = Scope::default();
    for stmt in body {
        scope.visit_stmt(stmt.clone());
    }
    scope
}

fn platform_value(expr: &Expr, scope: &Scope) -> Option<&'static str> {
    match scope.symbol(expr)?.as_str() {
        "sys.platform" => match std::env::consts::OS {
            "windows" => Some("win32"),
            "linux" => Some("linux"),
            "macos" => Some("darwin"),
            _ => None,
        },
        "os.name" => match std::env::consts::OS {
            "windows" => Some("nt"),
            "linux" | "macos" => Some("posix"),
            _ => None,
        },
        _ => None,
    }
}

fn platform_truth(expr: &Expr, scope: &Scope) -> Option<bool> {
    match expr {
        Expr::UnaryOp(node) if node.op == ast::UnaryOp::Not => {
            Some(!platform_truth(&node.operand, scope)?)
        }
        Expr::Compare(node) if node.ops.len() == 1 && node.comparators.len() == 1 => {
            let (actual, expected) = if let Some(actual) = platform_value(&node.left, scope) {
                (actual, string_constant(&node.comparators[0])?)
            } else {
                (
                    platform_value(&node.comparators[0], scope)?,
                    string_constant(&node.left)?,
                )
            };
            match node.ops[0] {
                ast::CmpOp::Eq => Some(actual == expected),
                ast::CmpOp::NotEq => Some(actual != expected),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Resolve only literal native-platform import guards whose sys/os binding is
/// proven. Unknown/shadowed conditions keep both alternatives and remain unknown.
/// This uses the actual analyzer platform; it does not infer a target from names
/// of files, classes, properties or the exception being documented.
pub(crate) fn active_module_statements(body: &[Stmt]) -> Vec<Stmt> {
    fn active(body: &[Stmt], scope: &Scope) -> Vec<Stmt> {
        let mut out = Vec::new();
        for stmt in body {
            if let Stmt::If(node) = stmt {
                if let Some(yes) = platform_truth(&node.test, scope) {
                    out.extend(active(if yes { &node.body } else { &node.orelse }, scope));
                    continue;
                }
            }
            out.push(stmt.clone());
        }
        out
    }
    active(body, &annotation_scope(body))
}

pub(crate) fn builtin_base(scope: &Scope, name: &str) -> Option<String> {
    let expr = Expr::parse(name, "<class-base>").ok()?;
    let symbol = scope.symbol(&expr)?;
    let leaf = symbol.strip_prefix("builtins.")?;
    (builtin_exception(leaf)
        || matches!(
            leaf,
            "object"
                | "type"
                | "dict"
                | "list"
                | "tuple"
                | "set"
                | "frozenset"
                | "str"
                | "int"
                | "float"
                | "complex"
                | "bytes"
                | "bytearray"
                | "bool"
                | "memoryview"
                | "property"
                | "staticmethod"
                | "classmethod"
        ))
    .then_some(symbol)
}

fn builtin_exception(name: &str) -> bool {
    matches!(
        name,
        "BaseException"
            | "BaseExceptionGroup"
            | "Exception"
            | "ExceptionGroup"
            | "ArithmeticError"
            | "AssertionError"
            | "AttributeError"
            | "BufferError"
            | "EOFError"
            | "ImportError"
            | "LookupError"
            | "MemoryError"
            | "NameError"
            | "OSError"
            | "ReferenceError"
            | "RuntimeError"
            | "StopAsyncIteration"
            | "StopIteration"
            | "SyntaxError"
            | "SystemError"
            | "TypeError"
            | "ValueError"
            | "Warning"
            | "FloatingPointError"
            | "OverflowError"
            | "ZeroDivisionError"
            | "ModuleNotFoundError"
            | "IndexError"
            | "KeyError"
            | "UnboundLocalError"
            | "BlockingIOError"
            | "ChildProcessError"
            | "ConnectionError"
            | "FileExistsError"
            | "FileNotFoundError"
            | "InterruptedError"
            | "IsADirectoryError"
            | "NotADirectoryError"
            | "PermissionError"
            | "ProcessLookupError"
            | "TimeoutError"
            | "BrokenPipeError"
            | "ConnectionAbortedError"
            | "ConnectionRefusedError"
            | "ConnectionResetError"
            | "NotImplementedError"
            | "RecursionError"
            | "PythonFinalizationError"
            | "IndentationError"
            | "TabError"
            | "UnicodeError"
            | "UnicodeDecodeError"
            | "UnicodeEncodeError"
            | "UnicodeTranslateError"
            | "DeprecationWarning"
            | "PendingDeprecationWarning"
            | "RuntimeWarning"
            | "SyntaxWarning"
            | "UserWarning"
            | "FutureWarning"
            | "ImportWarning"
            | "UnicodeWarning"
            | "BytesWarning"
            | "ResourceWarning"
            | "EncodingWarning"
            | "GeneratorExit"
            | "KeyboardInterrupt"
            | "SystemExit"
    )
}

fn annotation(expr: &Expr, scope: &Scope) -> Option<ExceptionType> {
    if let Some(text) = string_constant(expr) {
        let parsed = Expr::parse(text, "<exception-annotation>").ok()?;
        return annotation(&parsed, scope);
    }
    match expr {
        Expr::Constant(node) if matches!(node.value, ast::Constant::None) => Some(ExceptionType {
            names: Vec::new(),
            nullable: true,
        }),
        Expr::BinOp(node) if node.op == ast::Operator::BitOr => {
            let mut left = annotation(&node.left, scope)?;
            let right = annotation(&node.right, scope)?;
            left.names.extend(right.names);
            left.names.sort();
            left.names.dedup();
            left.nullable |= right.nullable;
            Some(left)
        }
        Expr::Subscript(node) => {
            let symbol = scope.symbol(&node.value)?;
            match symbol.as_str() {
                "typing.Optional" | "typing_extensions.Optional" => {
                    let mut ty = annotation(&node.slice, scope)?;
                    ty.nullable = true;
                    Some(ty)
                }
                "typing.Union" | "typing_extensions.Union" => {
                    let Expr::Tuple(tuple) = node.slice.as_ref() else {
                        return None;
                    };
                    let mut result = ExceptionType {
                        names: Vec::new(),
                        nullable: false,
                    };
                    for item in &tuple.elts {
                        let ty = annotation(item, scope)?;
                        result.names.extend(ty.names);
                        result.nullable |= ty.nullable;
                    }
                    result.names.sort();
                    result.names.dedup();
                    Some(result)
                }
                _ => None,
            }
        }
        _ => {
            let symbol = scope.symbol(expr)?;
            let name = symbol.strip_prefix("builtins.")?;
            builtin_exception(name).then(|| ExceptionType {
                names: vec![name.to_string()],
                nullable: false,
            })
        }
    }
}

pub(crate) type MemberTypes = HashMap<String, Option<ExceptionType>>;

pub(crate) fn class_members(
    module_scope: &Scope,
    class: &ast::StmtClassDef,
) -> (bool, MemberTypes) {
    let trusted = module_scope.proven(class.name.as_str()) == Some(&Binding::Class);
    let bases_proven = class.bases.iter().all(|base| {
        let base = match base {
            Expr::Subscript(node) => node.value.as_ref(),
            other => other,
        };
        let Some(name) = qualified_name(base) else {
            return false;
        };
        let root = name.split('.').next().unwrap_or(&name);
        matches!(
            module_scope.proven(root),
            Some(Binding::Class | Binding::Symbol(_))
        ) || (!module_scope.star
            && !module_scope.bindings.contains_key(root)
            && builtin_base(module_scope, &name).is_some())
    });
    let mut class_scope = Scope::default();
    for stmt in &class.body {
        class_scope.visit_stmt(stmt.clone());
    }
    let mut annotation_scope = module_scope.clone();
    for (name, bindings) in &class_scope.bindings {
        annotation_scope
            .bindings
            .insert(name.clone(), bindings.clone());
    }
    let mut members = MemberTypes::new();
    for (name, bindings) in &class_scope.bindings {
        if !bindings.is_empty() {
            members.insert(name.clone(), None);
        }
    }
    if trusted {
        for stmt in &class.body {
            let Stmt::FunctionDef(function) = stmt else {
                continue;
            };
            if class_scope
                .bindings
                .get(function.name.as_str())
                .is_none_or(|bindings| bindings.len() != 1)
            {
                continue;
            }
            if function.decorator_list.len() == 1
                && annotation_scope
                    .symbol(&function.decorator_list[0])
                    .as_deref()
                    == Some("builtins.property")
            {
                members.insert(
                    function.name.to_string(),
                    function
                        .returns
                        .as_deref()
                        .and_then(|expr| annotation(expr, &annotation_scope)),
                );
            }
        }
    }
    if !trusted {
        members.insert("<unproven-class-binding>".to_string(), None);
    }
    (bases_proven, members)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Instance(String),
    Exception(ExceptionType),
}

#[derive(Clone, Default)]
struct Flow {
    locals: HashMap<String, Value>,
    guards: HashMap<(String, String), ExceptionType>,
    written: HashSet<(String, String)>,
}

impl Flow {
    fn key(expr: &Expr) -> Option<(String, String)> {
        let Expr::Attribute(attr) = expr else {
            return None;
        };
        let Expr::Name(receiver) = attr.value.as_ref() else {
            return None;
        };
        Some((receiver.id.to_string(), attr.attr.to_string()))
    }
    fn value(&self, expr: &Expr, model: &DataclassModel) -> Option<Value> {
        match expr {
            Expr::Name(name) => self.locals.get(name.id.as_str()).cloned(),
            Expr::Attribute(_) => {
                let key = Self::key(expr)?;
                if self.written.contains(&key) {
                    return None;
                }
                let Value::Instance(class) = self.locals.get(&key.0)? else {
                    return None;
                };
                self.guards
                    .get(&key)
                    .cloned()
                    .or_else(|| model.exception_member_type(class, &key.1))
                    .map(Value::Exception)
            }
            _ => None,
        }
    }
    fn assign(&mut self, target: &Expr, value: Option<Value>) {
        match target {
            Expr::Name(name) => {
                self.locals.remove(name.id.as_str());
                self.guards
                    .retain(|(receiver, _), _| receiver != name.id.as_str());
                if let Some(value) = value {
                    self.locals.insert(name.id.to_string(), value);
                }
            }
            Expr::Attribute(_) => {
                if let Some(key) = Self::key(target) {
                    let instance = self.locals.get(&key.0).cloned();
                    let aliases = self
                        .locals
                        .iter()
                        .filter(|(_, value)| Some(*value) == instance.as_ref())
                        .map(|(name, _)| name.clone())
                        .collect::<Vec<_>>();
                    self.guards.remove(&key);
                    self.written.insert(key.clone());
                    for alias in aliases {
                        let other = (alias, key.1.clone());
                        self.guards.remove(&other);
                        self.written.insert(other);
                    }
                }
            }
            Expr::Tuple(tuple) => {
                for item in &tuple.elts {
                    self.assign(item, None);
                }
            }
            Expr::List(list) => {
                for item in &list.elts {
                    self.assign(item, None);
                }
            }
            _ => {}
        }
    }
    fn narrow(&mut self, expr: &Expr, nonnull: bool, model: &DataclassModel) {
        let Some(Value::Exception(mut ty)) = self.value(expr, model) else {
            return;
        };
        if nonnull {
            ty.nullable = false;
        } else {
            ty.names.clear();
            ty.nullable = true;
        }
        match expr {
            Expr::Name(name) => {
                self.locals
                    .insert(name.id.to_string(), Value::Exception(ty));
            }
            Expr::Attribute(_) => {
                if let Some(key) = Self::key(expr) {
                    self.guards.insert(key, ty);
                }
            }
            _ => {}
        }
    }
    fn merge(left: Option<Self>, right: Option<Self>) -> Option<Self> {
        match (left, right) {
            (None, other) | (other, None) => other,
            (Some(mut left), Some(right)) => {
                left.locals
                    .retain(|key, value| right.locals.get(key) == Some(value));
                left.guards
                    .retain(|key, value| right.guards.get(key) == Some(value));
                left.written.extend(right.written);
                Some(left)
            }
        }
    }
}

fn none_guard(expr: &Expr) -> Option<(&Expr, bool)> {
    let Expr::Compare(compare) = expr else {
        return None;
    };
    if compare.ops.len() != 1 || compare.comparators.len() != 1 {
        return None;
    }
    let is_none = |expr: &Expr| matches!(expr, Expr::Constant(node) if matches!(node.value, ast::Constant::None));
    let value = if is_none(&compare.comparators[0]) {
        compare.left.as_ref()
    } else if is_none(&compare.left) {
        &compare.comparators[0]
    } else {
        return None;
    };
    match compare.ops[0] {
        ast::CmpOp::IsNot => Some((value, true)),
        ast::CmpOp::Is => Some((value, false)),
        _ => None,
    }
}

#[derive(Default)]
struct WalrusTargets(Vec<Expr>);
impl Visitor for WalrusTargets {
    fn visit_expr_named_expr(&mut self, node: ast::ExprNamedExpr) {
        self.0.push(*node.target);
        self.visit_expr(*node.value);
    }
    fn visit_expr_lambda(&mut self, _node: ast::ExprLambda) {}
}

fn invalidate_walrus(flow: &mut Flow, expr: &Expr) {
    let mut targets = WalrusTargets::default();
    targets.visit_expr(expr.clone());
    for target in targets.0 {
        flow.assign(&target, None);
    }
}

fn function_binding(flow: &mut Flow, name: &str, args: &ast::Arguments, decorators: &[Expr]) {
    flow.locals.remove(name);
    flow.guards.retain(|(receiver, _), _| receiver != name);
    for expr in decorators {
        invalidate_walrus(flow, expr);
    }
    for arg in args
        .posonlyargs
        .iter()
        .chain(&args.args)
        .chain(&args.kwonlyargs)
    {
        if let Some(expr) = arg.default.as_deref() {
            invalidate_walrus(flow, expr);
        }
    }
}

fn walk(
    body: &[Stmt],
    mut flow: Flow,
    model: &DataclassModel,
    resolved: &mut HashMap<usize, Vec<String>>,
) -> Option<Flow> {
    for stmt in body {
        match stmt {
            Stmt::FunctionDef(node) => function_binding(
                &mut flow,
                node.name.as_str(),
                &node.args,
                &node.decorator_list,
            ),
            Stmt::AsyncFunctionDef(node) => function_binding(
                &mut flow,
                node.name.as_str(),
                &node.args,
                &node.decorator_list,
            ),
            Stmt::ClassDef(_) => flow = Flow::default(),
            Stmt::Raise(node) => {
                if let Some(expr) = node.exc.as_deref() {
                    if let Some(Value::Exception(ty)) = flow.value(expr, model) {
                        if !ty.nullable && !ty.names.is_empty() {
                            resolved.insert(expr.start().to_usize(), ty.names);
                        }
                    }
                }
                return None;
            }
            Stmt::Return(_) => return None,
            Stmt::If(node) => {
                invalidate_walrus(&mut flow, &node.test);
                let mut yes = flow.clone();
                let mut no = flow;
                if let Some((expr, nonnull)) = none_guard(&node.test) {
                    yes.narrow(expr, nonnull, model);
                    no.narrow(expr, !nonnull, model);
                }
                flow = Flow::merge(
                    walk(&node.body, yes, model, resolved),
                    walk(&node.orelse, no, model, resolved),
                )?;
            }
            Stmt::Assert(node) => {
                invalidate_walrus(&mut flow, &node.test);
                if let Some((expr, nonnull)) = none_guard(&node.test) {
                    flow.narrow(expr, nonnull, model);
                }
            }
            Stmt::Assign(node) => {
                invalidate_walrus(&mut flow, &node.value);
                let value = flow.value(&node.value, model);
                for target in &node.targets {
                    invalidate_walrus(&mut flow, target);
                    flow.assign(target, value.clone());
                }
            }
            Stmt::AnnAssign(node) => {
                if let Some(expr) = node.value.as_deref() {
                    invalidate_walrus(&mut flow, expr);
                }
                let value = node
                    .value
                    .as_deref()
                    .and_then(|expr| flow.value(expr, model));
                invalidate_walrus(&mut flow, &node.target);
                flow.assign(&node.target, value);
            }
            Stmt::AugAssign(node) => {
                invalidate_walrus(&mut flow, &node.value);
                invalidate_walrus(&mut flow, &node.target);
                flow.assign(&node.target, None);
            }
            Stmt::Delete(node) => {
                for target in &node.targets {
                    flow.assign(target, None);
                }
            }
            Stmt::Expr(node) => invalidate_walrus(&mut flow, &node.value),
            Stmt::Pass(_) => {}
            _ => {
                flow = Flow::default();
            }
        }
    }
    Some(flow)
}

pub(crate) fn resolved_exception_values(
    module_scope: &Scope,
    model: &DataclassModel,
    args: &ast::Arguments,
    body: &[Stmt],
    instance_class: Option<&str>,
) -> HashMap<usize, Vec<String>> {
    let mut flow = Flow::default();
    for arg in args
        .posonlyargs
        .iter()
        .chain(&args.args)
        .chain(&args.kwonlyargs)
    {
        if let Some(ty) = arg
            .def
            .annotation
            .as_deref()
            .and_then(|expr| annotation(expr, module_scope))
        {
            flow.locals
                .insert(arg.def.arg.to_string(), Value::Exception(ty));
        }
    }
    if let (Some(class), Some(arg)) = (
        instance_class,
        args.posonlyargs.iter().chain(&args.args).next(),
    ) {
        flow.locals
            .insert(arg.def.arg.to_string(), Value::Instance(class.to_string()));
    }
    let mut resolved = HashMap::new();
    walk(body, flow, model, &mut resolved);
    resolved
}

pub(crate) fn canonical_builtin_exception(module_scope: &Scope, name: &str) -> Option<String> {
    let expr = Expr::parse(name, "<documented-exception>").ok()?;
    let ty = annotation(&expr, module_scope)?;
    (!ty.nullable && ty.names.len() == 1).then(|| ty.names[0].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected_parent(source: &str) -> Option<String> {
        let suite = ast::Suite::parse(source, "platform-imports.py").expect("valid module");
        let active = active_module_statements(&suite);
        let scope = annotation_scope(&active);
        match scope.proven("Parent") {
            Some(Binding::Symbol(name)) => Some(name.clone()),
            _ => None,
        }
    }

    #[test]
    fn native_platform_imports_use_proven_sys_and_os_bindings() {
        for (import, condition, selected) in [
            (
                "import sys",
                "sys.platform == 'win32'",
                cfg!(target_os = "windows"),
            ),
            (
                "import sys as runtime",
                "runtime.platform != 'win32'",
                !cfg!(target_os = "windows"),
            ),
            ("import os", "os.name == 'nt'", cfg!(target_os = "windows")),
        ] {
            let source = format!("{import}\nif {condition}:\n    from .yes import Parent\nelse:\n    from .no import Parent\n");
            assert_eq!(
                selected_parent(&source).as_deref(),
                Some(if selected {
                    ".yes.Parent"
                } else {
                    ".no.Parent"
                })
            );
        }
    }

    #[test]
    fn shadowed_or_unknown_platform_conditions_keep_imports_unproven() {
        for prefix in [
            "",
            "import sys\nsys = other",
            "import sys\nsys.platform = 'win32'",
            "if flag:\n    import sys",
            "from somewhere import *\nimport sys",
        ] {
            let source = format!("{prefix}\nif sys.platform == 'win32':\n    from .yes import Parent\nelse:\n    from .no import Parent\n");
            assert_eq!(selected_parent(&source), None, "{prefix}");
        }
    }
}
