//! Strict-only documentation-presence diagnostics for private Python objects.
//!
//! Unlike pydoclint, these rules check that a docstring exists. They are
//! deliberately isolated from the existing SKD semantic checks and SK609.
//! The parser is the shared Python AST; we never infer definitions from regexes.

use crate::config::{name_matches_pattern, EffectiveConfig};
use crate::diagnostic::{Diagnostic, Span};
use crate::python_ast::{qualified_name, string_constant, suite_docstring, PythonAst};
use rustpython_parser::ast::{Expr, Stmt};
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Module,
    Class,
    Function,
}

/// Return diagnostics for SK625 through SK628 only.
///
/// Existing SKLint validation of `.pyi` is unchanged: the exclusion here only
/// applies to *new missing-docstring* requirements, mirroring Ruff's D1xx stub
/// exemption. `.ipynb` is never treated as a Python module for this check.
pub fn run_docstring_presence_rules(
    path: &Path,
    source: &str,
    config: &EffectiveConfig,
) -> Vec<Diagnostic> {
    if !config.strict
        || !["SK625", "SK626", "SK627", "SK628"]
            .iter()
            .any(|code| config.is_enabled(code))
        || matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("pyi" | "ipynb")
        )
    {
        return Vec::new();
    }

    let display_path = path.display().to_string();
    let Ok(ast) = PythonAst::parse(source, &display_path) else {
        // Do not guess structure in an AST-unsupported Python file. SK902/903
        // already report incomplete parsing instead of silently assuming valid.
        return Vec::new();
    };

    let aliases = imported_names(&ast.suite);
    let mut diagnostics = Vec::new();
    visit_suite(
        &ast.suite,
        Scope::Module,
        &ast,
        source,
        &display_path,
        &aliases,
        config,
        &mut diagnostics,
    );
    diagnostics
}

fn imported_names(suite: &[Stmt]) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for stmt in suite {
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    let full = alias.name.as_str();
                    let visible = alias
                        .asname
                        .as_ref()
                        .map(|name| name.as_str())
                        .unwrap_or_else(|| full.split('.').next().unwrap_or(full));
                    let target = if alias.asname.is_some() {
                        full
                    } else {
                        visible
                    };
                    result.insert(visible.to_string(), target.to_string());
                }
            }
            Stmt::ImportFrom(import) => {
                let Some(module) = import.module.as_ref() else {
                    continue;
                };
                for alias in &import.names {
                    if alias.name.as_str() == "*" {
                        continue;
                    }
                    let visible = alias.asname.as_ref().unwrap_or(&alias.name).to_string();
                    result.insert(visible, format!("{}.{}", module, alias.name));
                }
            }
            _ => {}
        }
    }
    result
}

fn canonical_decorator(expr: &Expr, aliases: &HashMap<String, String>) -> Option<String> {
    let name = qualified_name(expr)?;
    let (head, rest) = name
        .split_once('.')
        .map(|(head, rest)| (head, Some(rest)))
        .unwrap_or((name.as_str(), None));
    let Some(canonical) = aliases.get(head) else {
        return Some(name);
    };
    Some(match rest {
        Some(rest) => format!("{canonical}.{rest}"),
        None => canonical.clone(),
    })
}

fn ignored_decorators(
    decorators: &[Expr],
    aliases: &HashMap<String, String>,
    config: &EffectiveConfig,
) -> bool {
    decorators.iter().any(|decorator| {
        let Some(name) = canonical_decorator(decorator, aliases) else {
            return false;
        };
        matches!(
            name.as_str(),
            "typing.overload"
                | "typing_extensions.overload"
                | "typing.override"
                | "typing_extensions.override"
        ) || config
            .docstring_ignore_decorators
            .iter()
            .any(|pattern| name_matches_pattern(&name, pattern))
    })
}

fn is_property(decorators: &[Expr], aliases: &HashMap<String, String>) -> bool {
    decorators.iter().any(|decorator| {
        canonical_decorator(decorator, aliases).is_some_and(|name| {
            matches!(
                name.as_str(),
                "property" | "builtins.property" | "functools.cached_property"
            )
        })
    })
}

fn is_property_setter_or_deleter(decorators: &[Expr], aliases: &HashMap<String, String>) -> bool {
    decorators.iter().any(|decorator| {
        canonical_decorator(decorator, aliases)
            .is_some_and(|name| name.ends_with(".setter") || name.ends_with(".deleter"))
    })
}

fn is_private(name: &str) -> bool {
    name.len() > 1 && name.starts_with('_') && !name.ends_with("__")
}

fn is_private_constant(name: &str) -> bool {
    let body = name.trim_start_matches('_');
    is_private(name)
        && !body.is_empty()
        && body.chars().any(|ch| ch.is_ascii_uppercase())
        && body
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

fn existing_sk609_handles(stmt: &Stmt, ast: &PythonAst, source: &str) -> bool {
    // Existing SK609 is intentionally left unchanged in this candidate:
    // it recognizes exactly the textual `: Final` or `:Final` assignment.
    // Do not suppress SK628 for aliases or qualified `typing.Final` until
    // SK609 really supports them; otherwise they would silently go unchecked.
    let line = ast.location_of(stmt).line;
    source
        .lines()
        .nth(line.saturating_sub(1))
        .is_some_and(|text| text.contains(": Final") || text.contains(":Final"))
}

fn has_adjacent_constant_docstring(suite: &[Stmt], index: usize) -> bool {
    match suite.get(index + 1) {
        Some(Stmt::Expr(node)) => string_constant(&node.value).is_some(),
        _ => false,
    }
}

fn report(
    code: &str,
    message: String,
    stmt: &Stmt,
    ast: &PythonAst,
    source: &str,
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let loc = ast.location_of(stmt);
    let line = source.lines().nth(loc.line.saturating_sub(1)).unwrap_or("");
    // AST columns are byte-based; SKLint diagnostic columns use Unicode chars.
    let mut prefix_bytes = loc.column.saturating_sub(1).min(line.len());
    while !line.is_char_boundary(prefix_bytes) {
        prefix_bytes -= 1;
    }
    let column = line[..prefix_bytes].chars().count() + 1;
    diagnostics.push(Diagnostic::new(
        code,
        message,
        path,
        Span::new(loc.line, column, loc.line, column + 1),
        "warning",
    ));
}

#[allow(clippy::too_many_arguments)]
fn visit_function(
    name: &str,
    decorators: &[Expr],
    body: &[Stmt],
    stmt: &Stmt,
    parent: Scope,
    ast: &PythonAst,
    source: &str,
    path: &str,
    aliases: &HashMap<String, String>,
    config: &EffectiveConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if parent == Scope::Class
        && is_private(name)
        && suite_docstring(body).is_none()
        && !ignored_decorators(decorators, aliases, config)
        && !is_property_setter_or_deleter(decorators, aliases)
    {
        let property = is_property(decorators, aliases);
        let code = if property { "SK627" } else { "SK625" };
        if config.is_enabled(code) {
            let kind = if property { "property" } else { "method" };
            report(
                code,
                format!("Private {kind} `{name}` must have a docstring in strict mode"),
                stmt,
                ast,
                source,
                path,
                diagnostics,
            );
        }
    }
    visit_suite(
        body,
        Scope::Function,
        ast,
        source,
        path,
        aliases,
        config,
        diagnostics,
    );
}

#[allow(clippy::too_many_arguments)]
fn visit_suite(
    suite: &[Stmt],
    scope: Scope,
    ast: &PythonAst,
    source: &str,
    path: &str,
    aliases: &HashMap<String, String>,
    config: &EffectiveConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (index, stmt) in suite.iter().enumerate() {
        match stmt {
            Stmt::FunctionDef(node) => visit_function(
                node.name.as_str(),
                &node.decorator_list,
                &node.body,
                stmt,
                scope,
                ast,
                source,
                path,
                aliases,
                config,
                diagnostics,
            ),
            Stmt::AsyncFunctionDef(node) => visit_function(
                node.name.as_str(),
                &node.decorator_list,
                &node.body,
                stmt,
                scope,
                ast,
                source,
                path,
                aliases,
                config,
                diagnostics,
            ),
            Stmt::ClassDef(node) => {
                if is_private(node.name.as_str())
                    && config.is_enabled("SK626")
                    && suite_docstring(&node.body).is_none()
                    && !ignored_decorators(&node.decorator_list, aliases, config)
                {
                    report(
                        "SK626",
                        format!(
                            "Private class `{}` must have a docstring in strict mode",
                            node.name
                        ),
                        stmt,
                        ast,
                        source,
                        path,
                        diagnostics,
                    );
                }
                visit_suite(
                    &node.body,
                    Scope::Class,
                    ast,
                    source,
                    path,
                    aliases,
                    config,
                    diagnostics,
                );
            }
            Stmt::Assign(node) if scope != Scope::Function => {
                if config.is_enabled("SK628") && !has_adjacent_constant_docstring(suite, index) {
                    for target in &node.targets {
                        if let Expr::Name(name) = target {
                            if is_private_constant(name.id.as_str()) {
                                report(
                                    "SK628",
                                    format!(
                                        "Private constant `{}` must have a string description in strict mode",
                                        name.id
                                    ),
                                    stmt,
                                    ast,
                                    source,
                                    path,
                                    diagnostics,
                                );
                            }
                        }
                    }
                }
            }
            Stmt::AnnAssign(node) if scope != Scope::Function => {
                if config.is_enabled("SK628")
                    && node.value.is_some()
                    && !existing_sk609_handles(stmt, ast, source)
                    && !has_adjacent_constant_docstring(suite, index)
                {
                    if let Expr::Name(name) = node.target.as_ref() {
                        if is_private_constant(name.id.as_str()) {
                            report(
                                "SK628",
                                format!(
                                    "Private constant `{}` must have a string description in strict mode",
                                    name.id
                                ),
                                stmt,
                                ast,
                                source,
                                path,
                                diagnostics,
                            );
                        }
                    }
                }
            }
            // Definitions under TYPE_CHECKING and guarded imports remain in
            // the same module/class scope (no regex-based false nesting).
            Stmt::If(node) => {
                visit_suite(
                    &node.body,
                    scope,
                    ast,
                    source,
                    path,
                    aliases,
                    config,
                    diagnostics,
                );
                visit_suite(
                    &node.orelse,
                    scope,
                    ast,
                    source,
                    path,
                    aliases,
                    config,
                    diagnostics,
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FileInlineConfig, PyProjectConfig, VscodeConfig};

    fn check(source: &str, filename: &str, strict: bool, ignored: &[&str]) -> Vec<String> {
        let project = PyProjectConfig {
            has_sklint_section: true,
            strict: Some(strict),
            docstring_ignore_decorators: ignored.iter().map(|x| x.to_string()).collect(),
            ..PyProjectConfig::default()
        };
        let cfg = EffectiveConfig::resolve(
            &VscodeConfig::default(),
            &project,
            &FileInlineConfig::default(),
        );
        run_docstring_presence_rules(Path::new(filename), source, &cfg)
            .into_iter()
            .map(|d| d.code)
            .collect()
    }

    #[test]
    fn strict_only_covers_private_methods_classes_properties_and_constants() {
        let source = r#"
class _Private:
    def _method(self):
        pass

    @property
    def _value(self):
        return 1

_MAX_RETRIES = 3
"#;
        let c = check(source, "example.py", true, &[]);
        assert!(c.contains(&"SK625".to_string()));
        assert!(c.contains(&"SK626".to_string()));
        assert!(c.contains(&"SK627".to_string()));
        assert!(c.contains(&"SK628".to_string()));
        assert!(check(source, "example.py", false, &[]).is_empty());
    }

    #[test]
    fn documented_objects_do_not_trigger_missing_docstrings() {
        let source = r#"
class _Private:
    """Private class."""

    def _method(self):
        """Private method."""
        pass

    @property
    def _value(self):
        """Private property."""
        return 1

_MAX_RETRIES = 3
"""Private constant."""
"#;
        assert!(check(source, "example.py", true, &[]).is_empty());
    }

    #[test]
    fn overload_and_override_and_custom_exclusion_skip_only_target_methods() {
        let source = r#"
from typing import overload as overload_alias
from typing_extensions import override as override_alias
from custom.decorators import skip_docs as hidden
class Public:
    """Documented class."""

    @overload_alias
    def _dispatch(self, arg: int) -> int: ...

    @override_alias
    def _override(self):
        pass

    @hidden
    def _ignored(self):
        pass

    def _ordinary(self):
        pass
"#;
        let c = check(source, "example.py", true, &["custom.decorators.skip_docs"]);
        assert_eq!(c, vec!["SK625"]);
    }

    #[test]
    fn pyi_and_notebook_skip_new_rules_without_disabling_general_sklint() {
        let source = "class _Private:\n    def _m(self): pass\n";
        assert!(check(source, "example.pyi", true, &[]).is_empty());
        assert!(check(source, "example.ipynb", true, &[]).is_empty());
        assert!(check(source, "example.py", true, &[]).contains(&"SK626".to_string()));
    }

    #[test]
    fn sk609_takes_existing_final_while_aliases_remain_covered() {
        let bare_final = "from typing import Final\n_MAX: Final[int] = 3\n";
        let c = check(bare_final, "example.py", true, &[]);
        assert!(!c.contains(&"SK628".to_string()));
        let report = crate::analyzer::analyze(crate::analyzer::AnalysisInput {
            path: Path::new("example.py").to_path_buf(),
            source: format!("# sklint: strict\n{bare_final}"),
            vscode_config: VscodeConfig::default(),
        });
        assert!(report.diagnostics.iter().any(|d| d.code == "SK609"));
        assert!(report.diagnostics.iter().all(|d| d.code != "SK628"));

        // SK609 currently does not parse imported alias names. SK628 must
        // retain strict coverage rather than skipping these constants.
        let alias = "from typing import Final as F\n_MAX: F[int] = 3\n";
        assert!(check(alias, "example.py", true, &[]).contains(&"SK628".to_string()));
        assert!(check(
            "import typing\n_MAX: typing.Final[int] = 3\n",
            "example.py",
            true,
            &[]
        )
        .contains(&"SK628".to_string()));
    }

    #[test]
    fn setter_and_deleter_have_no_separate_docstring_requirement() {
        let source = r#"
class Public:
    """Public class."""
    @property
    def _value(self):
        """Private property."""
        return 1
    @_value.setter
    def _value(self, v):
        pass
    @_value.deleter
    def _value(self):
        pass
"#;
        assert!(check(source, "example.py", true, &[]).is_empty());
    }

    #[test]
    fn nested_private_local_function_is_not_a_method() {
        let source = r#"
def outer():
    def _local():
        pass
    return _local
"#;
        assert!(check(source, "example.py", true, &[]).is_empty());
    }

    #[test]
    fn module_and_class_private_constants_only_not_locals_or_dunders() {
        let source = r#"
class Example:
    """Example."""
    _LIMIT = 3
    __all__ = []
    def normal(self):
        _LOCAL = 42
        return _LOCAL
"#;
        let c = check(source, "example.py", true, &[]);
        assert_eq!(c, vec!["SK628"]);
    }
}
