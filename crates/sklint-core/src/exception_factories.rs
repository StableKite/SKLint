//! Resolve documented standard-library exception factories through lexical bindings.
//!
//! A spelling such as `WinError` is not evidence of a factory. Only an actual
//! import, or an unambiguous alias of that import, permits a result-type override.
//! Conflicting bindings remain unknown; ordinary Raises checks still apply.

use crate::lexical_bindings::{binding, Bindings, KnownBinding, ScopeDefinitions};
use crate::python_ast::PythonAst;
use rustpython_ast::Visitor;
use rustpython_parser::ast::{self, Expr, Ranged, Stmt};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Binding {
    CtypesModule,
    WinErrorFactory,
}

impl KnownBinding for Binding {
    fn imported(module: Option<&str>, name: &str, aliased: bool) -> Option<Self> {
        match module {
            None if name == "ctypes" || (!aliased && name.starts_with("ctypes.")) => {
                Some(Self::CtypesModule)
            }
            Some("ctypes") if name == "WinError" => Some(Self::WinErrorFactory),
            _ => None,
        }
    }

    fn attribute(self, name: &str) -> Option<Self> {
        (self == Self::CtypesModule && name == "WinError").then_some(Self::WinErrorFactory)
    }

    fn module(self) -> Option<&'static str> {
        (self == Self::CtypesModule).then_some("ctypes")
    }
}

struct ScopeResolver<'a> {
    bindings: Bindings<Binding>,
    closure: Bindings<Binding>,
    calls: &'a mut HashMap<usize, String>,
}

fn resolve_scope(
    body: &[Stmt],
    parent: &Bindings<Binding>,
    args: Option<&ast::Arguments>,
    class: bool,
    calls: &mut HashMap<usize, String>,
) {
    let bindings = ScopeDefinitions::collect(body, args).stable_bindings(parent);
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
