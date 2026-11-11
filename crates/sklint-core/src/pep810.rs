//! PEP 810 grammar over the real Python token stream, retaining original ranges.
use rustpython_parser::ast::{self, Ranged, Stmt};
use rustpython_parser::text_size::{TextRange, TextSize};
use rustpython_parser::{lexer, Mode, Parse, ParseError, ParseErrorType, Tok};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LazyImport {
    pub range: TextRange,
    pub keyword_range: TextRange,
}

pub(crate) fn parse(
    source: &str,
    filename: &str,
) -> Option<Result<(ast::Suite, Vec<LazyImport>), ParseError>> {
    let mut tokens = lexer::lex(source, Mode::Module).peekable();
    let mut imports = HashMap::new();
    let mut filtered = Vec::new();
    while let Some(item) = tokens.next() {
        if let Ok((Tok::Name { name }, range)) = &item {
            if name == "lazy" {
                if let Some(Ok((Tok::Import | Tok::From, next_range))) = tokens.peek() {
                    imports.insert(next_range.start(), *range);
                    continue;
                }
            }
        }
        // Keep lexical errors as well as tokens: a later real syntax error
        // must retain its own position, rather than the initial soft keyword.
        let lexical_error = item.is_err();
        filtered.push(item);
        if lexical_error {
            break;
        }
    }
    if imports.is_empty() {
        return None;
    }
    Some(
        ast::Suite::parse_tokens(filtered, filename).and_then(|mut suite| {
            let mut lazy_imports = Vec::with_capacity(imports.len());
            visit(&mut suite, false, &imports, &mut lazy_imports).map_err(|range| ParseError {
                error: ParseErrorType::InvalidToken,
                offset: range.start(),
                source_path: filename.to_owned(),
            })?;
            if lazy_imports.len() != imports.len() {
                return Err(ParseError {
                    error: ParseErrorType::InvalidToken,
                    offset: *imports.keys().min().unwrap(),
                    source_path: filename.to_owned(),
                });
            }
            Ok((suite, lazy_imports))
        }),
    )
}

fn visit(
    suite: &mut [Stmt],
    forbidden: bool,
    imports: &HashMap<TextSize, TextRange>,
    lazy_imports: &mut Vec<LazyImport>,
) -> Result<(), TextRange> {
    for statement in suite {
        match statement {
            Stmt::Import(node) => {
                if let Some(keyword_range) = imports.get(&node.start()) {
                    if forbidden {
                        return Err(*keyword_range);
                    }
                    node.range = TextRange::new(keyword_range.start(), node.end());
                    lazy_imports.push(LazyImport {
                        range: node.range,
                        keyword_range: *keyword_range,
                    });
                }
            }
            Stmt::ImportFrom(node) => {
                if let Some(keyword_range) = imports.get(&node.start()) {
                    let future = node
                        .module
                        .as_ref()
                        .is_some_and(|name| name.as_str() == "__future__")
                        && node.level.map(|level| level.to_usize()).unwrap_or(0) == 0;
                    if forbidden
                        || future
                        || node.names.iter().any(|alias| alias.name.as_str() == "*")
                    {
                        return Err(*keyword_range);
                    }
                    node.range = TextRange::new(keyword_range.start(), node.end());
                    lazy_imports.push(LazyImport {
                        range: node.range,
                        keyword_range: *keyword_range,
                    });
                }
            }
            Stmt::FunctionDef(n) => visit(&mut n.body, true, imports, lazy_imports)?,
            Stmt::AsyncFunctionDef(n) => visit(&mut n.body, true, imports, lazy_imports)?,
            Stmt::ClassDef(n) => visit(&mut n.body, true, imports, lazy_imports)?,
            Stmt::If(n) => {
                visit(&mut n.body, forbidden, imports, lazy_imports)?;
                visit(&mut n.orelse, forbidden, imports, lazy_imports)?;
            }
            Stmt::For(n) => {
                visit(&mut n.body, forbidden, imports, lazy_imports)?;
                visit(&mut n.orelse, forbidden, imports, lazy_imports)?;
            }
            Stmt::AsyncFor(n) => {
                visit(&mut n.body, true, imports, lazy_imports)?;
                visit(&mut n.orelse, true, imports, lazy_imports)?;
            }
            Stmt::While(n) => {
                visit(&mut n.body, forbidden, imports, lazy_imports)?;
                visit(&mut n.orelse, forbidden, imports, lazy_imports)?;
            }
            Stmt::With(n) => visit(&mut n.body, forbidden, imports, lazy_imports)?,
            Stmt::AsyncWith(n) => visit(&mut n.body, true, imports, lazy_imports)?,
            Stmt::Match(n) => {
                for case in &mut n.cases {
                    visit(&mut case.body, forbidden, imports, lazy_imports)?;
                }
            }
            Stmt::Try(n) => visit_try(
                &mut n.body,
                &mut n.orelse,
                &mut n.finalbody,
                &mut n.handlers,
                imports,
                lazy_imports,
            )?,
            Stmt::TryStar(n) => visit_try(
                &mut n.body,
                &mut n.orelse,
                &mut n.finalbody,
                &mut n.handlers,
                imports,
                lazy_imports,
            )?,
            _ => {}
        }
    }
    Ok(())
}

fn visit_try(
    body: &mut [Stmt],
    orelse: &mut [Stmt],
    finalbody: &mut [Stmt],
    handlers: &mut [ast::ExceptHandler],
    imports: &HashMap<TextSize, TextRange>,
    lazy_imports: &mut Vec<LazyImport>,
) -> Result<(), TextRange> {
    visit(body, true, imports, lazy_imports)?;
    visit(orelse, true, imports, lazy_imports)?;
    visit(finalbody, true, imports, lazy_imports)?;
    for handler in handlers {
        let ast::ExceptHandler::ExceptHandler(handler) = handler;
        visit(&mut handler.body, true, imports, lazy_imports)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::python_ast::PythonAst;
    use rustpython_parser::ast::{Expr, Ranged, Stmt};

    #[test]
    fn full_lazy_ast_retains_modifier_aliases_and_original_utf8_positions() {
        let source = "\"\"\"Русский текст\"\"\"\nlazy import os as operating\nlazy from collections.abc import (\n    Callable as Handler,\n)\ndef invoke(callback: Handler[[], None]) -> int:\n    callback()\n    return int(41)\n";
        let ast = PythonAst::parse(source, "lazy.py").expect("complete lazy AST");
        assert_eq!(ast.suite.len(), 4);
        assert_eq!(ast.lazy_imports.len(), 2);
        for (index, line) in [(1, 2), (2, 3)] {
            assert!(ast.is_lazy_import(&ast.suite[index]));
            assert_eq!(ast.location_of(&ast.suite[index]).line, line);
            assert_eq!(ast.location_of(&ast.suite[index]).column, 1);
            let modifier = ast.lazy_imports[index - 1].keyword_range;
            assert_eq!(
                &source[usize::from(modifier.start())..usize::from(modifier.end())],
                "lazy"
            );
        }
        let Stmt::Import(import) = &ast.suite[1] else {
            panic!("import")
        };
        assert_eq!(import.names[0].name.as_str(), "os");
        assert_eq!(
            import.names[0].asname.as_ref().unwrap().as_str(),
            "operating"
        );
        assert_eq!(ast.location_of(&import.names[0]).column, 13);
        let Stmt::ImportFrom(import) = &ast.suite[2] else {
            panic!("from")
        };
        assert_eq!(import.module.as_ref().unwrap().as_str(), "collections.abc");
        assert_eq!(import.names[0].asname.as_ref().unwrap().as_str(), "Handler");
        assert_eq!(ast.location_of(&import.names[0]).line, 4);
        assert_eq!(ast.location_of(&import.names[0]).column, 5);
        let Stmt::FunctionDef(function) = &ast.suite[3] else {
            panic!("function")
        };
        assert_eq!(function.body.len(), 2);
        let Stmt::Return(statement) = &function.body[1] else {
            panic!("return")
        };
        let Expr::Call(call) = statement.value.as_deref().unwrap() else {
            panic!("call")
        };
        let literal = &call.args[0];
        assert_eq!(ast.location_of(literal).line, 8);
        assert_eq!(ast.location_of(literal).column, 16);
        assert_eq!(
            &source[usize::from(literal.start())..usize::from(literal.end())],
            "41"
        );
    }

    #[test]
    fn lazy_soft_keyword_keeps_strings_comments_identifiers_and_semicolons() {
        let source = "lazy = 'lazy import never'\n# lazy from ignored import ghost\nlazy import math as lazy; lazy from math import sqrt\nvalue = lazy.sqrt(4)\n";
        let parsed = PythonAst::parse(source, "soft.py").unwrap();
        assert_eq!(parsed.suite.len(), 4);
        assert_eq!(parsed.lazy_imports.len(), 2);
        assert!(!parsed.is_lazy_import(&parsed.suite[0]));
        assert!(parsed.is_lazy_import(&parsed.suite[1]));
        assert!(parsed.is_lazy_import(&parsed.suite[2]));
        assert_eq!(parsed.location_of(&parsed.suite[1]).column, 1);
        assert_eq!(parsed.location_of(&parsed.suite[2]).column, 27);
        let plain =
            PythonAst::parse("lazy = 41\ndef f(lazy):\n    return lazy\n", "plain.py").unwrap();
        assert!(plain.lazy_imports.is_empty());
    }

    #[test]
    fn actual315_lazy_scope_and_wildcard_restrictions_are_preserved() {
        for source in [
            "def f():\n    lazy import math\n",
            "async def f():\n    lazy import math\n",
            "class C:\n    lazy import math\n",
            "try:\n    lazy import math\nexcept ImportError:\n    pass\n",
            "try:\n    pass\nexcept Exception:\n    lazy from math import sqrt\n",
            "try:\n    pass\nfinally:\n    lazy import math\n",
            "try:\n    pass\nexcept* Exception:\n    lazy import math\n",
            "lazy from math import *\n",
            "lazy from __future__ import annotations\n",
        ] {
            assert!(
                PythonAst::parse(source, "invalid-lazy.py").is_err(),
                "{source}"
            );
        }
        for source in [
            "if True:\n    lazy import math\n",
            "for item in ():\n    lazy from math import sqrt\n",
            "while False:\n    lazy import math\n",
            "with manager() as resource:\n    lazy import math\n",
            "lazy \\\nfrom math import (\n    sqrt as root,\n)\n",
            "lazy from .__future__ import custom\n",
            "match value:\n    case 1:\n        lazy import math\n",
        ] {
            let parsed = PythonAst::parse(source, "valid-lazy.py").expect(source);
            assert_eq!(parsed.lazy_imports.len(), 1);
        }
    }

    #[test]
    fn lazy_imports_compose_with_existing_modern_grammar_without_offset_shift() {
        let source = "lazy from typing import Callable\nclass Owner[T]:\n    pass\ndef invoke[T](value: T) -> T:\n    return value\n";
        let parsed = PythonAst::parse(source, "modern.py").unwrap();
        assert_eq!(parsed.lazy_imports.len(), 1);
        assert_eq!(parsed.suite.len(), 3);
        let Stmt::FunctionDef(function) = &parsed.suite[2] else {
            panic!("function")
        };
        assert_eq!(parsed.location_of(function).line, 4);
        let Stmt::Return(statement) = &function.body[0] else {
            panic!("return")
        };
        let value = statement.value.as_deref().unwrap();
        assert_eq!(parsed.location_of(value).line, 5);
        assert_eq!(parsed.location_of(value).column, 12);
        assert_eq!(
            &source[usize::from(value.start())..usize::from(value.end())],
            "value"
        );
    }

    #[test]
    fn a_later_syntax_error_retains_its_actual_token_offset() {
        let source = "lazy import math\nreturn = 41\n";
        let error = PythonAst::parse(source, "invalid.py").unwrap_err();
        assert_eq!(usize::from(error.offset), source.find('=').unwrap());
        let source = "lazy import math\ndef f():\n    lazy import sys\n";
        let error = PythonAst::parse(source, "scope.py").unwrap_err();
        assert_eq!(usize::from(error.offset), source.rfind("lazy").unwrap());
    }

    #[test]
    fn lexical_errors_terminate_without_reading_the_failed_lexer_again() {
        for source in ["value = '\n", "lazy import math\nvalue = '\n"] {
            let error = PythonAst::parse(source, "lexical.py").unwrap_err();
            assert!(usize::from(error.offset) >= source.find("value").unwrap());
        }
    }
}
