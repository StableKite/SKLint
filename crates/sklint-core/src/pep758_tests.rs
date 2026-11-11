use crate::analyzer::{analyze, AnalysisInput};
use crate::config::VscodeConfig;
use crate::formatter::format_source;
use crate::python_ast::PythonAst;
use rustpython_parser::ast::{self, ExceptHandler, Expr, Ranged, Stmt};
use std::path::PathBuf;

fn handlers(statement: &Stmt) -> &[ExceptHandler] {
    match statement {
        Stmt::Try(node) => &node.handlers,
        Stmt::TryStar(node) => &node.handlers,
        _ => panic!("expected a complete try statement"),
    }
}

#[test]
fn pep758_full_tuple_ast_retains_original_utf8_byte_ranges() {
    for keyword in ["except", "except*"] {
        for head in ["ValueError, TypeError", "(ValueError, TypeError)"] {
            let source = format!(
                "\"\"\"Русский текст\"\"\"\ntry:\n    int(\"a\")\n{keyword} {head}:\n    pass\n"
            );
            let parsed = PythonAst::parse(&source, "case.py").unwrap();
            assert_eq!(parsed.suite.len(), 2);
            let ExceptHandler::ExceptHandler(handler) = &handlers(&parsed.suite[1])[0];
            assert_eq!(parsed.location_of(handler).line, 4);
            assert_eq!(parsed.location_of(handler).column, 1);
            let Expr::Tuple(tuple) = handler.type_.as_deref().unwrap() else {
                panic!("the complete exception expression must be a tuple");
            };
            assert_eq!(tuple.elts.len(), 2);
            assert_eq!(tuple.ctx, ast::ExprContext::Load);
            for (expression, expected) in tuple.elts.iter().zip(["ValueError", "TypeError"]) {
                let Expr::Name(name) = expression else {
                    panic!("expected a real name expression");
                };
                assert_eq!(name.id.as_str(), expected);
                assert_eq!(
                    &source[usize::from(name.start())..usize::from(name.end())],
                    expected
                );
            }
            assert_eq!(handler.body.len(), 1);
            assert!(matches!(handler.body[0], Stmt::Pass(_)));
        }
    }
}

#[test]
fn pep758_expression_lists_cover_real_python_expression_grammar() {
    for keyword in ["except", "except*"] {
        for head in [
            "ValueError, TypeError, OSError",
            "ValueError, TypeError,",
            "ValueError,",
            "ValueError if flag else TypeError, OSError",
            "lambda: ValueError, TypeError",
            "Errors[\"a:b\"], TypeError",
            "(ValueError, TypeError), OSError",
            "ValueError, \\\n    TypeError",
        ] {
            let source = format!("try:\n    int(\"a\")\n{keyword} {head}:\n    pass\n");
            let parsed = PythonAst::parse(&source, "case.py")
                .unwrap_or_else(|error| panic!("{keyword} {head}: {error}"));
            let ExceptHandler::ExceptHandler(handler) = &handlers(&parsed.suite[0])[0];
            assert!(matches!(handler.type_.as_deref(), Some(Expr::Tuple(_))));
        }
    }
}

#[test]
fn pep758_as_retains_single_expression_and_parenthesized_tuple_rules() {
    for keyword in ["except", "except*"] {
        for head in ["(ValueError, TypeError) as exc", "ValueError as exc"] {
            let source = format!("try:\n    int(\"a\")\n{keyword} {head}:\n    pass\n");
            let parsed = PythonAst::parse(&source, "case.py").unwrap();
            let ExceptHandler::ExceptHandler(handler) = &handlers(&parsed.suite[0])[0];
            assert_eq!(handler.name.as_ref().unwrap().as_str(), "exc");
        }
        for head in [
            "ValueError, TypeError as exc",
            "ValueError, as exc",
            "ValueError TypeError",
            "ValueError, *errors",
        ] {
            let source = format!("try:\n    int(\"a\")\n{keyword} {head}:\n    pass\n");
            assert!(PythonAst::parse(&source, "invalid.py").is_err(), "{source}");
        }
    }
    assert!(PythonAst::parse("try:\n    pass\nexcept*:\n    pass\n", "invalid.py").is_err());
    assert!(PythonAst::parse("try:\n    pass\nexcept:\n    pass\n", "bare.py").is_ok());
}

#[test]
fn pep758_composes_with_complete_pep810_and_pep695_ast() {
    let source = "lazy import math\nclass Owner[T]:\n    pass\ntry:\n    int(\"a\")\nexcept ValueError, TypeError:\n    pass\n";
    let parsed = PythonAst::parse(source, "modern.py").unwrap();
    assert_eq!(parsed.lazy_imports.len(), 1);
    assert!(parsed.is_lazy_import(&parsed.suite[0]));
    assert!(matches!(parsed.suite[1], Stmt::ClassDef(_)));
    let ExceptHandler::ExceptHandler(handler) = &handlers(&parsed.suite[2])[0];
    assert!(matches!(handler.type_.as_deref(), Some(Expr::Tuple(_))));
    assert_eq!(parsed.location_of(handler).line, 6);
}

#[test]
fn pep758_preserves_semantic_diagnostics_and_maps_original_header_spans() {
    for keyword in ["except", "except*"] {
        let mut reports = Vec::new();
        for head in ["(ValueError, TypeError)", "ValueError, TypeError"] {
            // Numeric conversion exercises the full semantic analysis.
            // Every pre-existing policy diagnostic must remain.
            let source = format!("__all__ = (\"Guarded\",)\n\nclass Guarded:\n    def __init__(self) -> None:\n        try:\n            self.value = int(41)\n        {keyword} {head}:\n            self.value = int(41)\n");
            let header_line = source
                .lines()
                .position(|line| line.trim_start().starts_with(keyword))
                .unwrap()
                + 1;
            let report = analyze(AnalysisInput {
                path: PathBuf::from("case.py"),
                source,
                vscode_config: VscodeConfig {
                    strict: Some(true),
                    ..VscodeConfig::default()
                },
            });
            assert!(!report.diagnostics.iter().any(|d| d.code == "SKD002"));
            assert!(report.diagnostics.iter().any(|d| d.code == "SK901"));
            reports.push((report.diagnostics, header_line));
        }
        let (parenthesized, header_line) = &reports[0];
        let (unparenthesized, other_header_line) = &reports[1];
        assert_eq!(header_line, other_header_line);
        assert_eq!(parenthesized.len(), unparenthesized.len());
        for (old, new) in parenthesized.iter().zip(unparenthesized) {
            let mut expected = old.clone();
            if expected.code == "SK506" && expected.line == *header_line {
                // Keep the existing SK506 range convention, accounting exactly
                // for the two parentheses absent from the equivalent header.
                assert_eq!(expected.end_line, *header_line);
                assert!(expected.end_column > expected.column + 2);
                expected.end_column -= 2;
            }
            assert_eq!(&expected, new);
        }
    }
}

#[test]
fn pep758_formatter_accepts_both_styles_and_reaches_a_fixed_point() {
    for keyword in ["except", "except*"] {
        for head in ["(ValueError, TypeError)", "ValueError, TypeError"] {
            let source = format!("try:\n    int(\"a\")\n{keyword} {head}:\n    pass\n");
            let first = format_source(PathBuf::from("case.py"), source, VscodeConfig::default());
            assert!(!first.blocked_by_syntax);
            assert!(first.source.contains(&format!("{keyword} {head}:")));
            PythonAst::parse(&first.source, "formatted.py").unwrap();
            let second = format_source(
                PathBuf::from("case.py"),
                first.source.clone(),
                VscodeConfig::default(),
            );
            assert!(!second.blocked_by_syntax);
            assert_eq!(second.applied, 0);
            assert_eq!(second.source, first.source);
        }
    }
}

#[test]
fn pep758_formatter_stays_fail_closed_for_invalid_as_lists() {
    for keyword in ["except", "except*"] {
        let source =
            format!("try:\n    int(\"a\")\n{keyword} ValueError, TypeError as exc:\n    pass\n");
        let report = format_source(
            PathBuf::from("invalid.py"),
            source.clone(),
            VscodeConfig::default(),
        );
        assert!(report.blocked_by_syntax);
        assert_eq!(report.applied, 0);
        assert_eq!(report.source, source);
    }
}
