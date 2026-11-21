use crate::diagnostic::{Diagnostic, Span};
use crate::python_ast::PythonAst;
use rustpython_ast::Visitor;
use rustpython_parser::ast::{self, Ranged};
use rustpython_parser::text_size::TextRange;

pub(crate) fn check(mut ast: PythonAst, source: &str, path: &str) -> Vec<Diagnostic> {
    let suite = std::mem::take(&mut ast.suite);
    let mut rule = LazyImports {
        ast: &ast,
        source,
        path,
        diagnostics: Vec::new(),
        module_binding: true,
    };
    for import in &ast.lazy_imports {
        rule.report(
            import.keyword_range,
            "Lazy imports are forbidden in strict mode",
        );
    }
    for statement in suite {
        rule.visit_stmt(statement);
    }
    rule.diagnostics
}

struct LazyImports<'a> {
    ast: &'a PythonAst,
    source: &'a str,
    path: &'a str,
    diagnostics: Vec<Diagnostic>,
    module_binding: bool,
}

impl LazyImports<'_> {
    fn report(&mut self, range: TextRange, message: &str) {
        let (line, column) = self.position(range.start().into());
        let (end_line, end_column) = self.position(range.end().into());
        self.diagnostics.push(Diagnostic::new(
            "SK511",
            message,
            self.path,
            Span::new(line, column, end_line, end_column),
            "warning",
        ));
    }

    fn position(&self, offset: usize) -> (usize, usize) {
        let location = self.ast.scalar_location_of_offset(self.source, offset);
        (location.line, location.column)
    }

    fn scope(&mut self, body: Vec<ast::Stmt>) {
        let outer = self.module_binding;
        self.module_binding = body.iter().any(declares_global);
        for statement in body {
            self.visit_stmt(statement);
        }
        self.module_binding = outer;
    }
}

fn declares_global(stmt: &ast::Stmt) -> bool {
    use ast::Stmt;
    match stmt {
        Stmt::Global(node) => node
            .names
            .iter()
            .any(|name| name.as_str() == "__lazy_modules__"),
        Stmt::If(node) => node.body.iter().chain(&node.orelse).any(declares_global),
        Stmt::For(node) => node.body.iter().chain(&node.orelse).any(declares_global),
        Stmt::AsyncFor(node) => node.body.iter().chain(&node.orelse).any(declares_global),
        Stmt::While(node) => node.body.iter().chain(&node.orelse).any(declares_global),
        Stmt::With(node) => node.body.iter().any(declares_global),
        Stmt::AsyncWith(node) => node.body.iter().any(declares_global),
        Stmt::Try(node) => {
            declares_global_in_try(&node.body, &node.handlers, &node.orelse, &node.finalbody)
        }
        Stmt::TryStar(node) => {
            declares_global_in_try(&node.body, &node.handlers, &node.orelse, &node.finalbody)
        }
        Stmt::Match(node) => node
            .cases
            .iter()
            .any(|case| case.body.iter().any(declares_global)),
        _ => false,
    }
}

fn declares_global_in_try(
    body: &[ast::Stmt],
    handlers: &[ast::ExceptHandler],
    orelse: &[ast::Stmt],
    finalbody: &[ast::Stmt],
) -> bool {
    body.iter()
        .chain(orelse)
        .chain(finalbody)
        .any(declares_global)
        || handlers.iter().any(|handler| {
            let ast::ExceptHandler::ExceptHandler(handler) = handler;
            handler.body.iter().any(declares_global)
        })
}

impl Visitor for LazyImports<'_> {
    fn visit_expr_name(&mut self, node: ast::ExprName) {
        if self.module_binding
            && node.ctx == ast::ExprContext::Store
            && node.id.as_str() == "__lazy_modules__"
        {
            self.report(
                node.range(),
                "Module bindings of `__lazy_modules__` are forbidden in strict mode",
            );
        }
    }

    fn visit_stmt_ann_assign(&mut self, node: ast::StmtAnnAssign) {
        if let Some(value) = node.value {
            self.visit_expr(*node.target);
            self.visit_expr(*value);
        }
    }

    fn visit_stmt_function_def(&mut self, node: ast::StmtFunctionDef) {
        self.visit_arguments(*node.args);
        for decorator in node.decorator_list {
            self.visit_expr(decorator);
        }
        self.scope(node.body);
    }

    fn visit_stmt_async_function_def(&mut self, node: ast::StmtAsyncFunctionDef) {
        self.visit_arguments(*node.args);
        for decorator in node.decorator_list {
            self.visit_expr(decorator);
        }
        self.scope(node.body);
    }

    fn visit_stmt_class_def(&mut self, node: ast::StmtClassDef) {
        for expr in node.bases.into_iter().chain(node.decorator_list) {
            self.visit_expr(expr);
        }
        for keyword in node.keywords {
            self.visit_keyword(keyword);
        }
        self.scope(node.body);
    }

    fn visit_expr_lambda(&mut self, node: ast::ExprLambda) {
        self.visit_arguments(*node.args);
        // The body belongs to the lambda, which cannot declare a global.
    }

    fn visit_arguments(&mut self, args: ast::Arguments) {
        for parameter in args
            .posonlyargs
            .into_iter()
            .chain(args.args)
            .chain(args.kwonlyargs)
        {
            if let Some(default) = parameter.default {
                self.visit_expr(*default);
            }
        }
    }

    fn visit_comprehension(&mut self, comprehension: ast::Comprehension) {
        // The iteration target is local to the comprehension. A walrus in
        // its expressions binds in the containing scope.
        self.visit_expr(comprehension.iter);
        for condition in comprehension.ifs {
            self.visit_expr(condition);
        }
    }

    fn visit_keyword(&mut self, keyword: ast::Keyword) {
        self.visit_expr(keyword.value);
    }

    fn visit_withitem(&mut self, item: ast::WithItem) {
        self.visit_expr(item.context_expr);
        if let Some(variable) = item.optional_vars {
            self.visit_expr(*variable);
        }
    }

    fn visit_match_case(&mut self, case: ast::MatchCase) {
        if let Some(guard) = case.guard {
            self.visit_expr(*guard);
        }
        for statement in case.body {
            self.visit_stmt(statement);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::analyzer::{analyze, AnalysisInput, AnalysisReport};
    use crate::config::VscodeConfig;
    use crate::diagnostic::Diagnostic;
    use std::path::PathBuf;

    fn analyze_source(source: &str, strict: bool, explicitly_selected: bool) -> AnalysisReport {
        analyze(AnalysisInput {
            path: PathBuf::from("lazy_import_fixture.py"),
            source: source.to_owned(),
            vscode_config: VscodeConfig {
                strict: Some(strict),
                select: if explicitly_selected {
                    vec!["SK511".to_owned()]
                } else {
                    Vec::new()
                },
                ..VscodeConfig::default()
            },
        })
    }

    fn warnings(source: &str) -> Vec<Diagnostic> {
        analyze_source(source, true, false)
            .diagnostics
            .into_iter()
            .filter(|d| d.code == "SK511")
            .collect()
    }

    #[test]
    fn reports_each_native_modifier_with_exact_source_coordinates() {
        let source = "lazy import os as system\n'я'; lazy from pathlib import (\n    Path,\n)\n";
        let spans = warnings(source)
            .into_iter()
            .map(|d| {
                assert!(d.fix.is_none());
                (d.line, d.column, d.end_line, d.end_column)
            })
            .collect::<Vec<_>>();
        assert_eq!(spans, [(1, 1, 1, 5), (2, 6, 2, 10)]);
    }

    #[test]
    fn reports_module_bindings_including_control_flow_and_destructuring() {
        let source = "__lazy_modules__ = ('os',)
if configured:
    __lazy_modules__: tuple[str, ...] = ('pathlib',)
match choice:
    case 'custom':
        (other, __lazy_modules__) = (None, ('json',))
__lazy_modules__ += ('math',)
";
        let spans = warnings(source)
            .into_iter()
            .map(|d| (d.line, d.column, d.end_column))
            .collect::<Vec<_>>();
        assert_eq!(spans, [(1, 1, 17), (3, 5, 21), (6, 17, 33), (7, 1, 17)]);
    }

    #[test]
    fn leaves_examples_eager_imports_and_unrelated_names_alone() {
        let source = r#"
"""lazy import example
lazy from example import name
__lazy_modules__ = ('example',)
"""
# lazy import ignored
# __lazy_modules__ = ('ignored',)
from typing import TYPE_CHECKING
import importlib
from pathlib import Path
if TYPE_CHECKING:
    from example import Integration
dialect = importlib.import_module(dialect_name)
lazy = "lazy from example import name"
__lazy_modules__: tuple[str, ...]
names = config.__lazy_modules__
class Unrelated:
    __lazy_modules__ = ('class attribute',)
def local():
    __lazy_modules__ = ('local variable',)
"#;
        assert!(warnings(source).is_empty());
    }

    #[test]
    fn requires_strict_mode_even_with_an_explicit_selector() {
        for selected in [false, true] {
            let report = analyze_source(
                "lazy import os\n__lazy_modules__ = ('os',)\n",
                false,
                selected,
            );
            assert!(!report.config.is_enabled("SK511"));
            assert!(report.diagnostics.iter().all(|d| d.code != "SK511"));
        }
    }

    #[test]
    fn reports_walrus_loop_target_and_explicit_global_stores() {
        let source = r#"(__lazy_modules__ := ("fractions",))
for __lazy_modules__ in [("decimal",)]:
    pass

def configure():
    if enabled:
        global __lazy_modules__
    __lazy_modules__ = ("math",)

results = [(__lazy_modules__ := value) for value in values]
"#;
        let found: Vec<_> = warnings(source)
            .into_iter()
            .map(|d| (d.line, d.column))
            .collect();
        assert_eq!(found, [(1, 2), (2, 5), (8, 5), (10, 13)]);
    }

    #[test]
    fn does_not_treat_comprehension_and_nested_scope_targets_as_global() {
        let source = r#"results = [__lazy_modules__ for __lazy_modules__ in values]
setter = lambda: (__lazy_modules__ := ("local",))

def function():
    for __lazy_modules__ in values:
        pass
    class Local:
        __lazy_modules__ = ("class",)
    return Local
"#;
        assert!(warnings(source).is_empty());
    }

    #[test]
    fn honors_inline_suppression() {
        assert!(warnings("lazy import os  # noqa: SK511\n").is_empty());
    }
}
