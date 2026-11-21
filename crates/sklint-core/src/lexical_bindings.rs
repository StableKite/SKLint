//! Collect lexical names and resolve unambiguous imported standard-library bindings.

use rustpython_ast::Visitor;
use rustpython_parser::ast::{self, Expr, Stmt};
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
enum Definition<B> {
    Import(B),
    Alias(Expr),
    Unknown,
}

pub(crate) type Bindings<B> = HashMap<String, B>;

pub(crate) trait KnownBinding: Copy + Eq {
    fn imported(module: Option<&str>, name: &str, aliased: bool) -> Option<Self>;
    fn attribute(self, name: &str) -> Option<Self>;
    fn module(self) -> Option<&'static str>;
}

pub(crate) fn binding<B: KnownBinding>(expr: &Expr, bindings: &Bindings<B>) -> Option<B> {
    match expr {
        Expr::Name(name) => bindings.get(name.id.as_str()).copied(),
        Expr::Attribute(attr) => binding(&attr.value, bindings)?.attribute(attr.attr.as_str()),
        _ => None,
    }
}

pub(crate) struct ScopeDefinitions<B> {
    names: HashMap<String, Vec<Definition<B>>>,
    mutated_modules: HashSet<String>,
    star_import: bool,
}

impl<B: KnownBinding> ScopeDefinitions<B> {
    pub(crate) fn empty() -> Self {
        Self {
            names: HashMap::new(),
            mutated_modules: HashSet::new(),
            star_import: false,
        }
    }

    pub(crate) fn collect(body: &[Stmt], args: Option<&ast::Arguments>) -> Self {
        let mut definitions = Self::empty();
        if let Some(args) = args {
            definitions.parameters(args);
        }
        for stmt in body {
            definitions.visit_stmt(stmt.clone());
        }
        definitions
    }

    pub(crate) fn forget_writes(&self, bindings: &mut Bindings<B>) {
        if self.star_import {
            bindings.clear();
            return;
        }
        if !self.mutated_modules.is_empty() {
            let resolved = self.resolve(bindings, &HashSet::new());
            let mut modified = self.modified_modules(bindings);
            modified.extend(self.modified_modules(&resolved));
            bindings.retain(|_, value| {
                !value
                    .module()
                    .is_some_and(|module| modified.contains(module))
            });
        }
        self.forget_locals(bindings);
    }

    pub(crate) fn forget_locals(&self, bindings: &mut Bindings<B>) {
        bindings.retain(|name, _| !self.names.contains_key(name));
    }

    pub(crate) fn stable_bindings(&self, parent: &Bindings<B>) -> Bindings<B> {
        let known = self.resolve(parent, &HashSet::new());
        let modified = self.modified_modules(&known);
        if modified.is_empty() {
            known
        } else {
            self.resolve(parent, &modified)
        }
    }

    fn modified_modules(&self, bindings: &Bindings<B>) -> HashSet<&'static str> {
        self.mutated_modules
            .iter()
            .filter_map(|name| bindings.get(name)?.module())
            .collect()
    }

    fn define(&mut self, name: &str, definition: Definition<B>) {
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

    fn resolve(&self, parent: &Bindings<B>, disabled_modules: &HashSet<&str>) -> Bindings<B> {
        let enabled = |value: B| {
            !value
                .module()
                .is_some_and(|module| disabled_modules.contains(module))
        };
        if self.star_import {
            return Bindings::new();
        }
        let mut known = parent.clone();
        known.retain(|name, value| !self.names.contains_key(name) && enabled(*value));
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
                    if resolved.iter().all(|other| *other == Some(value)) && enabled(value) {
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

impl<B: KnownBinding> Visitor for ScopeDefinitions<B> {
    fn visit_stmt_import(&mut self, node: ast::StmtImport) {
        for item in node.names {
            let local = item
                .asname
                .as_ref()
                .map(|name| name.as_str())
                .unwrap_or_else(|| item.name.as_str().split('.').next().unwrap_or(""));
            let definition = B::imported(None, item.name.as_str(), item.asname.is_some())
                .map_or(Definition::Unknown, Definition::Import);
            self.define(local, definition);
        }
    }

    fn visit_stmt_import_from(&mut self, node: ast::StmtImportFrom) {
        let absolute_module = node
            .module
            .as_ref()
            .filter(|_| node.level.is_none_or(|level| level.to_usize() == 0));
        for item in node.names {
            if item.name.as_str() == "*" {
                self.star_import = true;
                continue;
            }
            let local = item.asname.as_ref().unwrap_or(&item.name);
            let definition = absolute_module
                .and_then(|module| {
                    B::imported(
                        Some(module.as_str()),
                        item.name.as_str(),
                        item.asname.is_some(),
                    )
                })
                .map_or(Definition::Unknown, Definition::Import);
            self.define(local.as_str(), definition);
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
