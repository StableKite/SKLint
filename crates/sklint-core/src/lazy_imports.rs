use crate::diagnostic::{Diagnostic, Span};
use crate::python_ast::PythonAst;
use rustpython_ast::Visitor;
use rustpython_parser::ast::{self, Expr, Ranged};
use rustpython_parser::text_size::TextRange;

pub(crate) fn check(mut ast: PythonAst, source: &str, path: &str) -> Vec<Diagnostic> {
    let suite = std::mem::take(&mut ast.suite);
    let mut rule = LazyImports {
        ast: &ast,
        source,
        path,
        diagnostics: Vec::new(),
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

    fn check_target(&mut self, target: &Expr) {
        match target {
            Expr::Name(name) if name.id.as_str() == "__lazy_modules__" => self.report(
                name.range(),
                "Module assignments to `__lazy_modules__` are forbidden in strict mode",
            ),
            Expr::Tuple(tuple) => {
                for item in &tuple.elts {
                    self.check_target(item);
                }
            }
            Expr::List(list) => {
                for item in &list.elts {
                    self.check_target(item);
                }
            }
            Expr::Starred(starred) => self.check_target(&starred.value),
            _ => {}
        }
    }
}

impl Visitor for LazyImports<'_> {
    fn visit_stmt_assign(&mut self, node: ast::StmtAssign) {
        for target in &node.targets {
            self.check_target(target);
        }
    }

    fn visit_stmt_ann_assign(&mut self, node: ast::StmtAnnAssign) {
        if node.value.is_some() {
            self.check_target(&node.target);
        }
    }

    fn visit_stmt_aug_assign(&mut self, node: ast::StmtAugAssign) {
        self.check_target(&node.target);
    }

    // Walk suites in the module's scope. Function and class bodies create
    // separate namespaces, and expressions do not declare the module setting.
    fn visit_stmt_function_def(&mut self, _: ast::StmtFunctionDef) {}
    fn visit_stmt_async_function_def(&mut self, _: ast::StmtAsyncFunctionDef) {}
    fn visit_stmt_class_def(&mut self, _: ast::StmtClassDef) {}
    fn visit_expr(&mut self, _: Expr) {}

    // RustPython's default visitor does not descend into match cases.
    fn visit_match_case(&mut self, case: ast::MatchCase) {
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
    fn honors_inline_suppression() {
        assert!(warnings("lazy import os  # noqa: SK511\n").is_empty());
    }
}
