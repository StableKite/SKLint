//! Resolve documented standard-library exception factories through lexical bindings.
//!
//! A spelling such as `WinError` is not evidence of a factory. Only an actual
//! import, or an unambiguous alias of that import, permits a result-type override.
//! Conflicting bindings remain unknown; ordinary Raises checks still apply.

use crate::python_ast::PythonAst;
use rustpython_ast::Visitor;
use rustpython_parser::ast::{self, Expr, Ranged, Stmt};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Binding {
    CtypesModule,
    WinErrorFactory,
}

#[derive(Clone)]
enum Definition {
    Import(Binding),
    Alias(Expr),
    Unknown,
}

type Bindings = HashMap<String, Binding>;

#[derive(Default)]
struct ScopeDefinitions {
    names: HashMap<String, Vec<Definition>>,
    mutated_modules: HashSet<String>,
    star_import: bool,
}

fn binding(expr: &Expr, bindings: &Bindings) -> Option<Binding> {
    match expr {
        Expr::Name(name) => bindings.get(name.id.as_str()).copied(),
        Expr::Attribute(attr)
            if attr.attr.as_str() == "WinError"
                && binding(&attr.value, bindings) == Some(Binding::CtypesModule) =>
        {
            Some(Binding::WinErrorFactory)
        }
        _ => None,
    }
}

impl ScopeDefinitions {
    fn define(&mut self, name: &str, definition: Definition) {
        self.names
            .entry(name.to_string())
            .or_default()
            .push(definition);
    }

    fn parameters(&mut self, args: &ast::Arguments) {
        for arg in args
            .posonlyargs
            .iter()
            .chain(&args.args)
            .chain(&args.kwonlyargs)
        {
            self.define(arg.def.arg.as_str(), Definition::Unknown);
        }
        for arg in args.vararg.iter().chain(&args.kwarg) {
            self.define(arg.arg.as_str(), Definition::Unknown);
        }
    }

    fn outer_expressions(&mut self, args: &ast::Arguments) {
        for arg in args
            .posonlyargs
            .iter()
            .chain(&args.args)
            .chain(&args.kwonlyargs)
        {
            if let Some(annotation) = &arg.def.annotation {
                self.visit_expr(*annotation.clone());
            }
            if let Some(default) = &arg.default {
                self.visit_expr(*default.clone());
            }
        }
    }

    fn resolve(&self, parent: &Bindings, disable_modules: bool) -> Bindings {
        if self.star_import {
            return Bindings::new();
        }
        let mut known = parent.clone();
        known.retain(|name, value| {
            !self.names.contains_key(name) && !(disable_modules && *value == Binding::CtypesModule)
        });
        for _ in 0..=self.names.len() {
            let previous = known.clone();
            for (name, definitions) in &self.names {
                let resolved = definitions
                    .iter()
                    .map(|definition| match definition {
                        Definition::Import(value) => Some(*value),
                        Definition::Alias(expr) => binding(expr, &previous),
                        Definition::Unknown => None,
                    })
                    .collect::<Vec<_>>();
                if let Some(value) = resolved.first().copied().flatten() {
                    if resolved.iter().all(|other| *other == Some(value))
                        && !(disable_modules && value == Binding::CtypesModule)
                    {
                        known.insert(name.clone(), value);
                    } else {
                        known.remove(name);
                    }
                } else {
                    known.remove(name);
                }
            }
            if previous == known {
                break;
            }
        }
        known
    }
}

impl Visitor for ScopeDefinitions {
    fn visit_stmt_import(&mut self, node: ast::StmtImport) {
        for item in node.names {
            let local = item
                .asname
                .as_ref()
                .map(|name| name.as_str())
                .unwrap_or_else(|| item.name.as_str().split('.').next().unwrap_or(""));
            let is_ctypes = item.name.as_str() == "ctypes"
                || (item.asname.is_none() && item.name.as_str().starts_with("ctypes."));
            self.define(
                local,
                if is_ctypes {
                    Definition::Import(Binding::CtypesModule)
                } else {
                    Definition::Unknown
                },
            );
        }
    }

    fn visit_stmt_import_from(&mut self, node: ast::StmtImportFrom) {
        let absolute_ctypes = node.module.as_ref().map(|name| name.as_str()) == Some("ctypes")
            && node
                .level
                .as_ref()
                .map(|value| value.to_usize())
                .unwrap_or(0)
                == 0;
        for item in node.names {
            if item.name.as_str() == "*" {
                self.star_import = true;
                continue;
            }
            let local = item.asname.as_ref().unwrap_or(&item.name);
            self.define(
                local.as_str(),
                if absolute_ctypes && item.name.as_str() == "WinError" {
                    Definition::Import(Binding::WinErrorFactory)
                } else {
                    Definition::Unknown
                },
            );
        }
    }

    fn visit_stmt_assign(&mut self, node: ast::StmtAssign) {
        for target in node.targets {
            if let Expr::Name(name) = target {
                self.define(name.id.as_str(), Definition::Alias(*node.value.clone()));
            } else {
                self.visit_expr(target);
            }
        }
        self.visit_expr(*node.value);
    }

    fn visit_stmt_ann_assign(&mut self, node: ast::StmtAnnAssign) {
        if let Expr::Name(name) = node.target.as_ref() {
            self.define(
                name.id.as_str(),
                node.value
                    .as_ref()
                    .map(|value| Definition::Alias(*value.clone()))
                    .unwrap_or(Definition::Unknown),
            );
        } else {
            self.visit_expr(*node.target);
        }
        self.visit_expr(*node.annotation);
        if let Some(value) = node.value {
            self.visit_expr(*value);
        }
    }

    fn visit_expr_name(&mut self, node: ast::ExprName) {
        if matches!(node.ctx, ast::ExprContext::Store | ast::ExprContext::Del) {
            self.define(node.id.as_str(), Definition::Unknown);
        }
    }

    fn visit_expr_attribute(&mut self, node: ast::ExprAttribute) {
        if matches!(node.ctx, ast::ExprContext::Store | ast::ExprContext::Del) {
            let mut root = node.value.as_ref();
            while let Expr::Attribute(attr) = root {
                root = &attr.value;
            }
            if let Expr::Name(name) = root {
                self.mutated_modules.insert(name.id.to_string());
            }
        }
        self.visit_expr(*node.value);
    }

    fn visit_stmt_function_def(&mut self, node: ast::StmtFunctionDef) {
        self.define(node.name.as_str(), Definition::Unknown);
        self.outer_expressions(&node.args);
        for decorator in node.decorator_list {
            self.visit_expr(decorator);
        }
    }

    fn visit_stmt_async_function_def(&mut self, node: ast::StmtAsyncFunctionDef) {
        self.define(node.name.as_str(), Definition::Unknown);
        self.outer_expressions(&node.args);
        for decorator in node.decorator_list {
            self.visit_expr(decorator);
        }
    }

    fn visit_stmt_class_def(&mut self, node: ast::StmtClassDef) {
        self.define(node.name.as_str(), Definition::Unknown);
        for expr in node.bases.into_iter().chain(node.decorator_list) {
            self.visit_expr(expr);
        }
    }

    fn visit_expr_lambda(&mut self, node: ast::ExprLambda) {
        self.outer_expressions(&node.args);
    }

    fn visit_excepthandler_except_handler(&mut self, node: ast::ExceptHandlerExceptHandler) {
        if let Some(name) = &node.name {
            self.define(name.as_str(), Definition::Unknown);
        }
        self.generic_visit_excepthandler_except_handler(node);
    }

    fn visit_withitem(&mut self, node: ast::WithItem) {
        self.visit_expr(node.context_expr);
        if let Some(vars) = node.optional_vars {
            self.visit_expr(*vars);
        }
    }

    fn visit_match_case(&mut self, node: ast::MatchCase) {
        self.visit_pattern(node.pattern);
        if let Some(guard) = node.guard {
            self.visit_expr(*guard);
        }
        for stmt in node.body {
            self.visit_stmt(stmt);
        }
    }

    fn visit_pattern_match_as(&mut self, node: ast::PatternMatchAs) {
        if let Some(name) = &node.name {
            self.define(name.as_str(), Definition::Unknown);
        }
        self.generic_visit_pattern_match_as(node);
    }

    fn visit_pattern_match_star(&mut self, node: ast::PatternMatchStar) {
        if let Some(name) = &node.name {
            self.define(name.as_str(), Definition::Unknown);
        }
    }

    fn visit_pattern_match_mapping(&mut self, node: ast::PatternMatchMapping) {
        if let Some(name) = &node.rest {
            self.define(name.as_str(), Definition::Unknown);
        }
        self.generic_visit_pattern_match_mapping(node);
    }

    fn visit_comprehension(&mut self, node: ast::Comprehension) {
        // Comprehension targets belong to their own scope; walrus expressions
        // in the iterable/filters still bind in the enclosing function.
        self.visit_expr(node.iter);
        for condition in node.ifs {
            self.visit_expr(condition);
        }
    }

    // Declarations alone do not rebind a name. An actual assignment/import in
    // this scope is still collected and invalidates a conflicting binding.
    fn visit_stmt_global(&mut self, _node: ast::StmtGlobal) {}

    fn visit_stmt_nonlocal(&mut self, _node: ast::StmtNonlocal) {}
}

struct ScopeResolver<'a> {
    bindings: Bindings,
    closure: Bindings,
    calls: &'a mut HashMap<usize, String>,
}

fn resolve_scope(
    body: &[Stmt],
    parent: &Bindings,
    args: Option<&ast::Arguments>,
    class: bool,
    calls: &mut HashMap<usize, String>,
) {
    let mut definitions = ScopeDefinitions::default();
    if let Some(args) = args {
        definitions.parameters(args);
    }
    for stmt in body {
        definitions.visit_stmt(stmt.clone());
    }
    let mut bindings = definitions.resolve(parent, false);
    if definitions
        .mutated_modules
        .iter()
        .any(|name| bindings.get(name) == Some(&Binding::CtypesModule))
    {
        // Mutation through any module alias invalidates every module alias.
        bindings = definitions.resolve(parent, true);
    }
    let closure = if class {
        parent.clone()
    } else {
        bindings.clone()
    };
    let mut resolver = ScopeResolver {
        bindings,
        closure,
        calls,
    };
    for stmt in body {
        resolver.visit_stmt(stmt.clone());
    }
}

impl Visitor for ScopeResolver<'_> {
    fn visit_stmt_raise(&mut self, node: ast::StmtRaise) {
        if let Some(exc) = &node.exc {
            if let Expr::Call(call) = exc.as_ref() {
                if binding(&call.func, &self.bindings) == Some(Binding::WinErrorFactory) {
                    self.calls
                        .insert(u32::from(exc.start()) as usize, "OSError".to_string());
                }
            }
        }
    }

    fn visit_stmt_function_def(&mut self, node: ast::StmtFunctionDef) {
        resolve_scope(
            &node.body,
            &self.closure,
            Some(&node.args),
            false,
            self.calls,
        );
    }

    fn visit_stmt_async_function_def(&mut self, node: ast::StmtAsyncFunctionDef) {
        resolve_scope(
            &node.body,
            &self.closure,
            Some(&node.args),
            false,
            self.calls,
        );
    }

    fn visit_stmt_class_def(&mut self, node: ast::StmtClassDef) {
        resolve_scope(&node.body, &self.closure, None, true, self.calls);
    }

    fn visit_match_case(&mut self, node: ast::MatchCase) {
        for stmt in node.body {
            self.visit_stmt(stmt);
        }
    }
}

pub(crate) fn resolved_factory_calls(ast: &PythonAst) -> HashMap<usize, String> {
    let mut calls = HashMap::new();
    resolve_scope(&ast.suite, &Bindings::new(), None, false, &mut calls);
    calls
}
