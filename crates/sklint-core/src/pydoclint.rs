//! Rust-native pydoclint-compatible semantic rules.
//!
//! All Python ownership/control-flow/signature information comes from
//! `rustpython-parser`.  Docstring text is parsed separately because NumPy,
//! Google and Sphinx formats are documentation mini-languages rather than
//! Python syntax.

use crate::config::{EffectiveConfig, PydoclintCliOverrides};
use crate::dataclass_model::{DataclassField, DataclassModel};
use crate::diagnostic::{Diagnostic, Fix, Span};
use crate::pydoclint_doc::{
    find_top_level_colon, is_sphinx_param_key, normalize_doc_name, normalize_type_text,
    parse_docstring, parse_docstring_with_style_detection, parse_sphinx_field,
    sphinx_attribute_directive_name, split_google_name_and_type, DocItem, DocStyle,
    ParsedDocstring,
};
use crate::python_ast::{
    expression_text, function_facts, source_text, string_constant, suite_docstring,
    suite_docstring_stmt, FunctionFacts, PythonAst,
};
use rustpython_parser::ast::{self, Constant, Expr, Stmt};
use rustpython_parser::Parse;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoqaLocation {
    Docstring,
    Definition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PydoclintOptions {
    pub style: DocStyle,
    pub arg_type_hints_in_signature: bool,
    pub arg_type_hints_in_docstring: bool,
    pub check_arg_order: bool,
    pub skip_checking_short_docstrings: bool,
    pub skip_checking_raises: bool,
    pub skip_checking_private_functions: bool,
    pub allow_init_docstring: bool,
    pub check_return_types: bool,
    pub check_yield_types: bool,
    pub ignore_underscore_args: bool,
    pub ignore_private_args: bool,
    pub check_class_attributes: bool,
    pub should_document_private_class_attributes: bool,
    pub treat_property_methods_as_class_attributes: bool,
    pub only_attrs_with_classvar_are_treated_as_class_attrs: bool,
    pub require_inline_class_var_docs: bool,
    pub require_return_section_when_returning_nothing: bool,
    pub require_yield_section_when_yielding_nothing: bool,
    pub should_document_star_arguments: bool,
    pub omit_stars_when_documenting_varargs: bool,
    pub should_declare_assert_error_if_assert_statement_exists: bool,
    pub check_style_mismatch: bool,
    pub check_arg_defaults: bool,
    pub native_mode_noqa_location: NoqaLocation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PydoclintNativeOptions {
    pub quiet: bool,
    pub exclude: String,
    pub baseline: Option<PathBuf>,
    pub generate_baseline: bool,
    pub auto_regenerate_baseline: bool,
    pub show_filenames_in_every_violation_message: bool,
    /// True when the filename-output mode was explicitly configured. SKLint
    /// keeps its normal one-diagnostic-per-line product output unless the
    /// pydoclint-compatible mode is requested explicitly.
    pub show_filenames_configured: bool,
}

impl Default for PydoclintNativeOptions {
    fn default() -> Self {
        Self {
            quiet: false,
            exclude: r"\.git|\.tox".to_string(),
            baseline: None,
            generate_baseline: false,
            auto_regenerate_baseline: true,
            show_filenames_in_every_violation_message: false,
            show_filenames_configured: false,
        }
    }
}

impl PydoclintNativeOptions {
    pub fn load_for_path(path: &Path) -> Self {
        let mut options = Self::default();
        let Some(pyproject) = find_pyproject(path) else {
            return options;
        };
        let Ok(text) = fs::read_to_string(pyproject) else {
            return options;
        };
        apply_native_toml_section(&mut options, &text, "tool.pydoclint");
        apply_native_toml_section(&mut options, &text, "tool.sklint.pydoclint");
        options
    }

    pub fn apply_toml_text(&mut self, text: &str) {
        apply_native_toml_section(self, text, "tool.pydoclint");
        apply_native_toml_section(self, text, "tool.sklint.pydoclint");
    }
}

impl Default for PydoclintOptions {
    fn default() -> Self {
        Self {
            style: DocStyle::Numpy,
            arg_type_hints_in_signature: true,
            arg_type_hints_in_docstring: true,
            check_arg_order: true,
            skip_checking_short_docstrings: true,
            skip_checking_raises: false,
            skip_checking_private_functions: false,
            allow_init_docstring: false,
            check_return_types: true,
            check_yield_types: true,
            ignore_underscore_args: true,
            ignore_private_args: false,
            check_class_attributes: true,
            should_document_private_class_attributes: false,
            treat_property_methods_as_class_attributes: false,
            only_attrs_with_classvar_are_treated_as_class_attrs: false,
            require_inline_class_var_docs: false,
            require_return_section_when_returning_nothing: false,
            require_yield_section_when_yielding_nothing: false,
            should_document_star_arguments: true,
            omit_stars_when_documenting_varargs: false,
            should_declare_assert_error_if_assert_statement_exists: false,
            check_style_mismatch: false,
            check_arg_defaults: false,
            native_mode_noqa_location: NoqaLocation::Docstring,
        }
    }
}

impl PydoclintOptions {
    pub fn load_for_path(path: &Path) -> Self {
        Self::load_for_path_with_config(
            path,
            DocStyle::Numpy,
            None,
            &PydoclintCliOverrides::default(),
        )
    }

    pub fn load_for_path_with_fallback(path: &Path, fallback_style: DocStyle) -> Self {
        Self::load_for_path_with_config(
            path,
            fallback_style,
            None,
            &PydoclintCliOverrides::default(),
        )
    }

    pub fn load_for_path_with_config(
        path: &Path,
        fallback_style: DocStyle,
        explicit_config: Option<&Path>,
        overrides: &PydoclintCliOverrides,
    ) -> Self {
        Self::load_for_path_with_config_context(
            path,
            fallback_style,
            None,
            explicit_config,
            overrides,
        )
    }

    pub fn load_for_path_with_config_context(
        path: &Path,
        fallback_style: DocStyle,
        inferred_config_context: Option<&Path>,
        explicit_config: Option<&Path>,
        overrides: &PydoclintCliOverrides,
    ) -> Self {
        let mut options = Self {
            style: fallback_style,
            ..Self::default()
        };
        let project_context = inferred_config_context.unwrap_or(path);
        if let Some(pyproject) = find_pyproject(project_context) {
            if let Ok(text) = fs::read_to_string(pyproject) {
                options.apply_toml_text(&text);
            }
        }
        if let Some(explicit_config) = explicit_config {
            if let Ok(text) = fs::read_to_string(explicit_config) {
                options.apply_toml_text(&text);
            }
        }
        options.apply_cli_overrides(overrides);
        options
    }

    pub fn apply_toml_text(&mut self, text: &str) {
        apply_toml_section(self, text, "tool.pydoclint");
        apply_toml_section(self, text, "tool.sklint.pydoclint");
    }

    pub fn apply_cli_overrides(&mut self, overrides: &PydoclintCliOverrides) {
        macro_rules! apply {
            ($field:ident) => {
                if let Some(value) = overrides.$field {
                    self.$field = value;
                }
            };
        }

        apply!(style);
        apply!(arg_type_hints_in_signature);
        apply!(arg_type_hints_in_docstring);
        apply!(check_arg_order);
        apply!(skip_checking_short_docstrings);
        apply!(skip_checking_raises);
        apply!(skip_checking_private_functions);
        apply!(allow_init_docstring);
        apply!(check_return_types);
        apply!(check_yield_types);
        apply!(ignore_underscore_args);
        apply!(ignore_private_args);
        apply!(check_class_attributes);
        apply!(should_document_private_class_attributes);
        apply!(treat_property_methods_as_class_attributes);
        apply!(only_attrs_with_classvar_are_treated_as_class_attrs);
        apply!(require_inline_class_var_docs);
        apply!(require_return_section_when_returning_nothing);
        apply!(require_yield_section_when_yielding_nothing);
        apply!(should_document_star_arguments);
        apply!(omit_stars_when_documenting_varargs);
        apply!(should_declare_assert_error_if_assert_statement_exists);
        apply!(check_style_mismatch);
        apply!(check_arg_defaults);
        if let Some(value) = overrides.native_mode_noqa_location.as_deref() {
            self.native_mode_noqa_location = if value.eq_ignore_ascii_case("definition") {
                NoqaLocation::Definition
            } else {
                NoqaLocation::Docstring
            };
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.style == DocStyle::Sphinx && self.check_arg_defaults {
            return Err(
                "the option --check-arg-defaults is not compatible with --style=sphinx; this feature only applies to numpy and Google style docstrings"
                    .to_string(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum ParentDef<'a> {
    Module,
    Class(&'a ast::StmtClassDef),
    Function,
}

#[derive(Clone, Copy)]
enum FunctionRef<'a> {
    Sync(&'a ast::StmtFunctionDef),
    Async(&'a ast::StmtAsyncFunctionDef),
}

impl<'a> FunctionRef<'a> {
    fn name(self) -> &'a str {
        match self {
            Self::Sync(node) => node.name.as_str(),
            Self::Async(node) => node.name.as_str(),
        }
    }

    fn args(self) -> &'a ast::Arguments {
        match self {
            Self::Sync(node) => &node.args,
            Self::Async(node) => &node.args,
        }
    }

    fn body(self) -> &'a [Stmt] {
        match self {
            Self::Sync(node) => &node.body,
            Self::Async(node) => &node.body,
        }
    }

    fn decorators(self) -> &'a [Expr] {
        match self {
            Self::Sync(node) => &node.decorator_list,
            Self::Async(node) => &node.decorator_list,
        }
    }

    fn returns(self) -> Option<&'a Expr> {
        match self {
            Self::Sync(node) => node.returns.as_deref(),
            Self::Async(node) => node.returns.as_deref(),
        }
    }

    fn line(self, ast: &PythonAst) -> usize {
        match self {
            Self::Sync(node) => ast.location_of(node).line,
            Self::Async(node) => ast.location_of(node).line,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActualArg {
    name: String,
    ty: String,
}

#[derive(Debug, Clone)]
struct DocInfo {
    parsed: ParsedDocstring,
    closing_line: usize,
    style_mismatch: bool,
}

struct Visitor<'a> {
    display_path: String,
    source: &'a str,
    ast: &'a PythonAst,
    config: &'a EffectiveConfig,
    options: PydoclintOptions,
    dataclasses: DataclassModel,
    allow_structural_fixes: bool,
    diagnostics: Vec<Diagnostic>,
}

pub fn run_pydoclint_rules(path: &Path, source: &str, config: &EffectiveConfig) -> Vec<Diagnostic> {
    if !config
        .active_codes
        .iter()
        .any(|code| code.starts_with("SKD"))
    {
        return Vec::new();
    }

    let display_path = path.display().to_string();
    let mut sanitized_source = None;
    let ast = match PythonAst::parse(source, &display_path) {
        Ok(ast) => ast,
        Err(error) => {
            // CPython reports invisible Unicode characters using a dedicated
            // SyntaxError spelling. RustPython's ParseError wording is
            // different, so detect the same compatibility case from the source
            // transformation itself rather than from parser error text.
            let sanitized = replace_invisible_chars(source);
            if sanitized != source {
                match PythonAst::parse(&sanitized, &display_path) {
                    Ok(ast) => {
                        sanitized_source = Some(sanitized);
                        ast
                    }
                    Err(second_error) => {
                        if !config.is_enabled("SKD002") {
                            return Vec::new();
                        }
                        return vec![Diagnostic::new(
                            "SKD002",
                            format!(
                                "Syntax errors; cannot parse this Python file. Error message: {second_error}"
                            ),
                            display_path,
                            Span::new(0, 1, 0, 1),
                            "warning",
                        )];
                    }
                }
            } else {
                if !config.is_enabled("SKD002") {
                    return Vec::new();
                }
                return vec![Diagnostic::new(
                    "SKD002",
                    format!("Syntax errors; cannot parse this Python file. Error message: {error}"),
                    display_path,
                    Span::new(0, 1, 0, 1),
                    "warning",
                )];
            }
        }
    };

    let analysis_source = sanitized_source.as_deref().unwrap_or(source);
    let dataclasses = DataclassModel::from_path(path, analysis_source, &ast);
    let mut visitor = Visitor {
        display_path,
        source: analysis_source,
        ast: &ast,
        config,
        options: PydoclintOptions::load_for_path_with_config_context(
            path,
            config.formatter_docstring_style,
            config.pydoclint_inferred_config_context.as_deref(),
            config.pydoclint_config_path.as_deref(),
            &config.pydoclint_overrides,
        ),
        dataclasses,
        allow_structural_fixes: sanitized_source.is_none(),
        diagnostics: Vec::new(),
    };
    visitor.visit_statements(&visitor.ast.suite, ParentDef::Module);
    visitor.diagnostics
}

fn replace_invisible_chars(text: &str) -> String {
    text.chars()
        .filter_map(|ch| match ch {
            '\u{feff}' => Some(' '),
            '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{180e}' | '\u{061c}'
            | '\u{200e}' | '\u{200f}' | '\u{202a}' | '\u{202b}' | '\u{202c}' | '\u{202d}'
            | '\u{202e}' | '\u{2061}' | '\u{2062}' | '\u{2063}' | '\u{2064}' | '\u{034f}' => None,
            _ => Some(ch),
        })
        .collect()
}

impl<'a> Visitor<'a> {
    fn visit_statements(&mut self, body: &'a [Stmt], parent: ParentDef<'a>) {
        for stmt in body {
            match stmt {
                Stmt::ClassDef(class) => {
                    self.check_class(class);
                    self.visit_statements(&class.body, ParentDef::Class(class));
                }
                Stmt::FunctionDef(function) => {
                    let function = FunctionRef::Sync(function);
                    if !self.should_skip_function(function, parent) {
                        self.check_function(function, parent);
                        self.visit_statements(function.body(), ParentDef::Function);
                    }
                }
                Stmt::AsyncFunctionDef(function) => {
                    let function = FunctionRef::Async(function);
                    if !self.should_skip_function(function, parent) {
                        self.check_function(function, parent);
                        self.visit_statements(function.body(), ParentDef::Function);
                    }
                }
                Stmt::If(node) => {
                    self.visit_statements(&node.body, parent);
                    self.visit_statements(&node.orelse, parent);
                }
                Stmt::For(node) => {
                    self.visit_statements(&node.body, parent);
                    self.visit_statements(&node.orelse, parent);
                }
                Stmt::AsyncFor(node) => {
                    self.visit_statements(&node.body, parent);
                    self.visit_statements(&node.orelse, parent);
                }
                Stmt::While(node) => {
                    self.visit_statements(&node.body, parent);
                    self.visit_statements(&node.orelse, parent);
                }
                Stmt::With(node) => self.visit_statements(&node.body, parent),
                Stmt::AsyncWith(node) => self.visit_statements(&node.body, parent),
                Stmt::Match(node) => {
                    for case in &node.cases {
                        self.visit_statements(&case.body, parent);
                    }
                }
                Stmt::Try(node) => {
                    self.visit_statements(&node.body, parent);
                    for handler in &node.handlers {
                        let ast::ExceptHandler::ExceptHandler(handler) = handler;
                        self.visit_statements(&handler.body, parent);
                    }
                    self.visit_statements(&node.orelse, parent);
                    self.visit_statements(&node.finalbody, parent);
                }
                Stmt::TryStar(node) => {
                    self.visit_statements(&node.body, parent);
                    for handler in &node.handlers {
                        let ast::ExceptHandler::ExceptHandler(handler) = handler;
                        self.visit_statements(&handler.body, parent);
                    }
                    self.visit_statements(&node.orelse, parent);
                    self.visit_statements(&node.finalbody, parent);
                }
                _ => {}
            }
        }
    }

    fn should_skip_function(&self, function: FunctionRef<'a>, _parent: ParentDef<'a>) -> bool {
        self.options.skip_checking_private_functions && is_private_name(function.name())
    }

    fn check_function(&mut self, function: FunctionRef<'a>, parent: ParentDef<'a>) {
        let line = function.line(self.ast);
        let is_constructor = matches!(parent, ParentDef::Class(_)) && function.name() == "__init__";
        if is_constructor && !self.is_last_constructor(function, parent) {
            return;
        }

        let mut effective_doc = self.doc_info(function.body(), line);
        if is_constructor {
            effective_doc = self.constructor_doc(function, parent, effective_doc);
        }
        let Some(doc) = effective_doc else {
            return;
        };

        if doc.parsed.parse_error.is_some() {
            self.push(
                "SKD001",
                line,
                format!(
                    "Function/method `{}`: Potential formatting errors in docstring. Error message: {} (Note: DOC001 could trigger other unrelated violations under this function/method too. Please fix the docstring formatting first.)",
                    function.name(),
                    doc.parsed.parse_error.as_deref().unwrap_or("unknown parse error")
                ),
                self.suppression_line(line, Some(doc.closing_line)),
            );
            return;
        }

        if doc.style_mismatch {
            self.push(
                "SKD003",
                line,
                format!(
                    "Function/method `{}`: Docstring style mismatch. (Please read more at https://jsh9.github.io/pydoclint/style_mismatch.html ). You specified \"{}\" style, but the docstring is likely not written in this style.",
                    function.name(),
                    doc_style_name(self.options.style)
                ),
                self.suppression_line(line, Some(doc.closing_line)),
            );
        }

        if self.options.skip_checking_short_docstrings && doc.parsed.is_short {
            if is_constructor {
                self.check_constructor_return_sections(parent, &doc);
            }
            return;
        }

        self.check_arguments(function, parent, &doc);

        if !doc.style_mismatch {
            let mut facts = function_facts(function.body());
            // pydoclint 0.9.1 intentionally recognizes only statement-form
            // `yield` / `yield from` (an `ast.Expr` wrapping Yield/YieldFrom).
            // Keep the shared AST facts Python-correct for other SKLint rules,
            // but narrow the pydoclint view here for compatibility.
            facts.has_yield = pydoclint_has_yield_statements(function.body());
            if facts.has_yield && facts.has_return {
                self.check_mixed_return_yield(function, parent, &facts, &doc);
            } else {
                self.check_returns(function, parent, &facts, &doc);
                self.check_yields(function, parent, &facts, &doc);
            }
            if !self.options.skip_checking_raises {
                self.check_raises(function, parent, &facts, &doc);
            }
        }

        if is_constructor {
            self.check_constructor_return_sections(parent, &doc);
        }
    }

    fn check_arguments(&mut self, function: FunctionRef<'a>, parent: ParentDef<'a>, doc: &DocInfo) {
        let line = function.line(self.ast);
        let suppression = self.suppression_line(line, Some(doc.closing_line));
        let mut actual = collect_function_args(self.source, function, parent, &self.options);
        let mut documented = doc.parsed.params.clone();

        if !self.options.should_document_star_arguments {
            actual.retain(|arg| !arg.name.starts_with('*'));
        } else if self.options.omit_stars_when_documenting_varargs {
            add_stars_to_documented_args(&mut documented, &actual);
        }

        let prefix = function_prefix(function, parent);
        // Upstream 0.9.1 exits argument checks entirely when both lists are
        // empty. In particular, a normal zero-argument function must not
        // receive DOC106 merely because `--arg-type-hints-in-signature=True`.
        if documented.is_empty() && actual.is_empty() {
            return;
        }
        if documented.len() < actual.len() {
            let fix = (!doc.parsed.has_args_section && !actual.is_empty())
                .then(|| render_parameter_section(self.options.style, &actual, false))
                .and_then(|section| self.append_docstring_section_fix(function.body(), &section));
            self.push_with_fix(
                "SKD101",
                line,
                format!("{prefix}: Docstring contains fewer arguments than in function signature."),
                suppression,
                fix,
            );
        }
        if documented.len() > actual.len() {
            self.push(
                "SKD102",
                line,
                format!("{prefix}: Docstring contains more arguments than in function signature."),
                suppression,
            );
        }

        let consider_sig_types = self.options.arg_type_hints_in_signature && !doc.style_mismatch;
        let consider_doc_types = self.options.arg_type_hints_in_docstring && !doc.style_mismatch;
        let actual_has_any = actual.iter().any(|arg| !arg.ty.is_empty());
        let actual_has_all = actual
            .iter()
            .filter(|arg| !arg.name.starts_with('*'))
            .all(|arg| !arg.ty.is_empty());
        let doc_has_any = documented.iter().any(|arg| !arg.ty.is_empty());
        let doc_has_all = documented
            .iter()
            .filter(|arg| !arg.name.starts_with('*'))
            .all(|arg| !arg.ty.is_empty());

        if consider_sig_types && !actual_has_any {
            self.push(
                "SKD106",
                line,
                format!("{prefix}: The option `--arg-type-hints-in-signature` is `True` but there are no argument type hints in the signature"),
                suppression,
            );
        }
        if consider_sig_types && !actual_has_all {
            self.push(
                "SKD107",
                line,
                format!("{prefix}: The option `--arg-type-hints-in-signature` is `True` but not all args in the signature have type hints"),
                suppression,
            );
        }
        if !self.options.arg_type_hints_in_signature && !doc.style_mismatch && actual_has_any {
            self.push(
                "SKD108",
                line,
                format!("{prefix}: The option `--arg-type-hints-in-signature` is `False` but there are argument type hints in the signature"),
                suppression,
            );
        }
        if consider_doc_types && !documented.is_empty() && !doc_has_any {
            let desired = documented
                .iter()
                .filter_map(|doc_arg| {
                    let actual_arg = actual.iter().find(|arg| arg.name == doc_arg.name)?;
                    (!actual_arg.ty.is_empty())
                        .then(|| (doc_arg.name.clone(), actual_arg.ty.clone()))
                })
                .collect::<Vec<_>>();
            let fix = (desired.len() == documented.len())
                .then(|| self.rewrite_named_docstring_types_fix(function.body(), false, &desired))
                .flatten();
            self.push_with_fix(
                "SKD109",
                line,
                format!("{prefix}: The option `--arg-type-hints-in-docstring` is `True` but there are no type hints in the docstring arg list"),
                suppression,
                fix,
            );
        }
        if consider_doc_types && !doc_has_all {
            let fix = if doc_has_any {
                let missing = documented
                    .iter()
                    .filter(|arg| !arg.name.starts_with('*') && arg.ty.is_empty())
                    .collect::<Vec<_>>();
                let desired = missing
                    .iter()
                    .filter_map(|doc_arg| {
                        let actual_arg = actual.iter().find(|arg| arg.name == doc_arg.name)?;
                        (!actual_arg.ty.is_empty())
                            .then(|| (doc_arg.name.clone(), actual_arg.ty.clone()))
                    })
                    .collect::<Vec<_>>();
                (desired.len() == missing.len())
                    .then(|| {
                        self.rewrite_named_docstring_types_fix(function.body(), false, &desired)
                    })
                    .flatten()
            } else {
                None
            };
            self.push_with_fix(
                "SKD110",
                line,
                format!("{prefix}: The option `--arg-type-hints-in-docstring` is `True` but not all args in the docstring arg list have type hints"),
                suppression,
                fix,
            );
        }
        if !self.options.arg_type_hints_in_docstring && !doc.style_mismatch && doc_has_any {
            let desired = documented
                .iter()
                .filter(|arg| !arg.ty.is_empty())
                .map(|arg| (arg.name.clone(), String::new()))
                .collect::<Vec<_>>();
            let fix = self.rewrite_named_docstring_types_fix(function.body(), false, &desired);
            self.push_with_fix(
                "SKD111",
                line,
                format!("{prefix}: The option `--arg-type-hints-in-docstring` is `False` but there are type hints in the docstring arg list"),
                suppression,
                fix,
            );
        }

        if self.config.is_enabled("SKD608") {
            let empty_descriptions = documented
                .iter()
                .filter(|arg| arg.description.trim().is_empty())
                .map(|arg| arg.name.clone())
                .collect::<Vec<_>>();
            if !empty_descriptions.is_empty() {
                self.push(
                    "SKD608",
                    line,
                    format!(
                        "{prefix}: Docstring arguments must have non-empty descriptions. Empty: [{}].",
                        empty_descriptions.join(", ")
                    ),
                    suppression,
                );
            }
        }

        let actual_names = actual
            .iter()
            .map(|arg| arg.name.as_str())
            .collect::<Vec<_>>();
        let doc_names = documented
            .iter()
            .map(|arg| arg.name.as_str())
            .collect::<Vec<_>>();
        let same_names_ordered = actual_names == doc_names;
        let same_names_unordered = same_string_set_with_equal_len(&actual_names, &doc_names);

        if !same_names_unordered {
            let mut missing = actual
                .iter()
                .filter(|arg| !documented.iter().any(|doc_arg| doc_arg.name == arg.name))
                .map(format_actual_arg)
                .collect::<Vec<_>>();
            let mut extra = documented
                .iter()
                .filter(|arg| !actual.iter().any(|actual_arg| actual_arg.name == arg.name))
                .map(format_doc_item)
                .collect::<Vec<_>>();
            missing.sort();
            extra.sort();
            let mut postfix = Vec::new();
            if !missing.is_empty() {
                postfix.push(format!(
                    "Arguments in the function signature but not in the docstring: [{}].",
                    missing.join(", ")
                ));
            }
            if !extra.is_empty() {
                postfix.push(format!(
                    "Arguments in the docstring but not in the function signature: [{}].",
                    extra.join(", ")
                ));
            }
            self.push(
                "SKD103",
                line,
                format!(
                    "{prefix}: Docstring arguments are different from function arguments. (Or could be other formatting issues: https://jsh9.github.io/pydoclint/violation_codes.html#notes-on-doc103 ). {}",
                    postfix.join(" ")
                ),
                suppression,
            );
            return;
        }

        if self.options.check_arg_order && !same_names_ordered {
            let fix = self.reorder_docstring_items_fix(function.body(), false, &actual_names);
            self.push_with_fix(
                "SKD104",
                line,
                format!("{prefix}: Arguments are the same in the docstring and the function signature, but are in a different order."),
                suppression,
                fix,
            );
        }

        if (consider_sig_types && consider_doc_types)
            || (self.options.check_arg_order && !same_names_ordered)
        {
            let mismatches = actual
                .iter()
                .filter_map(|arg| {
                    let doc_arg = documented.iter().find(|doc_arg| doc_arg.name == arg.name)?;
                    (!types_equal(&arg.ty, &doc_arg.ty)).then(|| arg.name.clone())
                })
                .collect::<Vec<_>>();
            if !mismatches.is_empty() {
                let desired = actual
                    .iter()
                    .filter(|arg| mismatches.contains(&arg.name) && !arg.ty.is_empty())
                    .map(|arg| (arg.name.clone(), arg.ty.clone()))
                    .collect::<Vec<_>>();
                let fix = (consider_sig_types
                    && consider_doc_types
                    && desired.len() == mismatches.len())
                .then(|| self.rewrite_named_docstring_types_fix(function.body(), false, &desired))
                .flatten();
                let message = if self.options.check_arg_defaults {
                    // `Violation.appendMoreMsg()` inserts a separating space
                    // before this postfix, while the postfix itself starts
                    // with a dot. Preserve the resulting upstream quirk:
                    // `arg1, arg2 . (Note: ...)`.
                    format!(
                        "{prefix}: Argument names match, but type hints in these args do not match: {} . (Note: docstring arg defaults should look like: `, default=XXX`)",
                        mismatches.join(", ")
                    )
                } else {
                    format!(
                        "{prefix}: Argument names match, but type hints in these args do not match: {}",
                        mismatches.join(", ")
                    )
                };
                self.push_with_fix("SKD105", line, message, suppression, fix);
            }
        }
    }

    fn check_returns(
        &mut self,
        function: FunctionRef<'a>,
        parent: ParentDef<'a>,
        facts: &FunctionFacts,
        doc: &DocInfo,
    ) {
        if function.name() == "__init__" && matches!(parent, ParentDef::Class(_)) {
            return;
        }
        let line = function.line(self.ast);
        let suppression = self.suppression_line(line, Some(doc.closing_line));
        let prefix = function_prefix(function, parent);
        let return_expr = function.returns();
        let has_annotation = return_expr.is_some();
        let generator_kind = return_expr.and_then(generator_annotation_kind);
        let iterable =
            return_expr.is_some_and(|expr| is_iterator_or_iterable_annotation(self.source, expr));
        let only_yield = facts.has_yield && !facts.has_return;
        let property = is_property(function);

        if !doc.parsed.has_returns_section
            && !property
            && !(only_yield && iterable)
            && (facts.has_return || (has_annotation && generator_kind.is_none()))
            && (self.options.require_return_section_when_returning_nothing
                || !return_annotation_is_none_or_noreturn(return_expr, self.source))
        {
            let fix = return_expr
                .map(|expr| canonical_annotation_text(self.source, expr))
                .filter(|ty| !ty.is_empty())
                .map(|ty| render_value_section(self.options.style, false, &ty))
                .and_then(|section| self.append_docstring_section_fix(function.body(), &section));
            self.push_with_fix(
                "SKD201",
                line,
                format!("{prefix} does not have a return section in docstring"),
                suppression,
                fix,
            );
        }

        if doc.parsed.has_returns_section && !(facts.has_return || has_annotation) {
            self.push(
                "SKD202",
                line,
                format!(
                    "{prefix} has a return section in docstring, but there are no return statements or annotations"
                ),
                suppression,
            );
        }

        if !self.options.check_return_types {
            return;
        }
        let annotation = return_expr
            .map(|expr| return_annotation_text(self.source, expr))
            .unwrap_or_default();
        if doc.parsed.returns.is_empty()
            && ((matches!(annotation.as_str(), "None" | "NoReturn")
                && !self.options.require_return_section_when_returning_nothing)
                || generator_kind.is_some()
                || iterable
                || property)
        {
            return;
        }
        if let Some(postfix) =
            return_type_mismatch_message(self.options.style, &annotation, &doc.parsed.returns)
        {
            let fix = (!annotation.is_empty())
                .then(|| self.rewrite_value_docstring_type_fix(function.body(), false, &annotation))
                .flatten();
            self.push_with_fix(
                "SKD203",
                line,
                format!(
                    "{prefix} return type(s) in docstring not consistent with the return annotation. {postfix}"
                ),
                suppression,
                fix,
            );
        }
    }

    fn check_yields(
        &mut self,
        function: FunctionRef<'a>,
        parent: ParentDef<'a>,
        facts: &FunctionFacts,
        doc: &DocInfo,
    ) {
        let line = function.line(self.ast);
        let suppression = self.suppression_line(line, Some(doc.closing_line));
        let prefix = function_prefix(function, parent);
        let return_expr = function.returns();
        let generator_kind = return_expr.and_then(generator_annotation_kind);
        let iterable =
            return_expr.is_some_and(|expr| is_iterator_or_iterable_annotation(self.source, expr));
        let yield_type = return_expr.and_then(|expr| extract_yield_type(self.source, expr));

        if !doc.parsed.has_yields_section
            && facts.has_yield
            && !(yield_type.as_deref() == Some("None")
                && !self.options.require_yield_section_when_yielding_nothing)
        {
            let fix = ((generator_kind.is_some() || iterable) && yield_type.is_some())
                .then(|| {
                    render_value_section(
                        self.options.style,
                        true,
                        yield_type.as_deref().unwrap_or_default(),
                    )
                })
                .and_then(|section| self.append_docstring_section_fix(function.body(), &section));
            self.push_with_fix(
                    "SKD402",
                    line,
                    format!("{prefix} has \"yield\" statements, but the docstring does not have a \"Yields\" section"),
                    suppression,
                    fix,
                );
        }

        if doc.parsed.has_yields_section
            && (!facts.has_yield || (generator_kind.is_none() && !iterable))
            && !is_abstract(function)
        {
            self.push(
                "SKD403",
                line,
                format!(
                    "{prefix} has a \"Yields\" section in the docstring, but there are no \"yield\" statements, or the return annotation is not a Generator/Iterator/Iterable. (Or it could be because the function lacks a return annotation.)"
                ),
                suppression,
            );
        }

        if facts.has_yield && self.options.check_yield_types {
            let recognized = generator_kind.is_some() || iterable;
            let original_annotation = recognized
                .then(|| return_expr.map(|expr| return_annotation_text(self.source, expr)))
                .flatten();
            if let Some(postfix) = yield_type_mismatch_message(
                original_annotation.as_deref(),
                yield_type.as_deref(),
                &doc.parsed.yields,
                recognized,
                self.options.require_yield_section_when_yielding_nothing,
            ) {
                let fix = yield_type
                    .as_deref()
                    .filter(|ty| !ty.is_empty())
                    .and_then(|ty| {
                        self.rewrite_value_docstring_type_fix(function.body(), true, ty)
                    });
                self.push_with_fix(
                    "SKD404",
                    line,
                    format!(
                        "{prefix} yield type(s) in docstring not consistent with the return annotation. {postfix}"
                    ),
                    suppression,
                    fix,
                );
            }
        }
    }

    fn check_mixed_return_yield(
        &mut self,
        function: FunctionRef<'a>,
        parent: ParentDef<'a>,
        facts: &FunctionFacts,
        doc: &DocInfo,
    ) {
        if function.name() == "__init__" && matches!(parent, ParentDef::Class(_)) {
            return;
        }
        let line = function.line(self.ast);
        let suppression = self.suppression_line(line, Some(doc.closing_line));
        let prefix = function_prefix(function, parent);
        let return_expr = function.returns();
        let generator_kind = return_expr.and_then(generator_annotation_kind);
        let iterable =
            return_expr.is_some_and(|expr| is_iterator_or_iterable_annotation(self.source, expr));
        let has_annotation = return_expr.is_some();

        if !doc.parsed.has_returns_section && !facts.has_bare_return {
            let to_document = return_expr.and_then(|expr| {
                extract_generator_return_type(self.source, expr)
                    .or_else(|| Some(return_annotation_text(self.source, expr)))
            });
            if (facts.has_return || (has_annotation && generator_kind.is_none()))
                && (self.options.require_return_section_when_returning_nothing
                    || !matches!(to_document.as_deref(), Some("None") | Some("NoReturn")))
            {
                let fix = to_document
                    .as_deref()
                    .filter(|ty| !ty.is_empty())
                    .map(|ty| render_value_section(self.options.style, false, ty))
                    .and_then(|section| {
                        self.append_docstring_section_fix(function.body(), &section)
                    });
                self.push_with_fix(
                    "SKD201",
                    line,
                    format!("{prefix} does not have a return section in docstring"),
                    suppression,
                    fix,
                );
            }
        } else if doc.parsed.has_returns_section && self.options.check_return_types {
            if generator_kind.is_some() {
                let expected = return_expr
                    .and_then(|expr| extract_generator_return_type(self.source, expr))
                    .unwrap_or_default();
                if let Some(postfix) =
                    return_type_mismatch_message(self.options.style, &expected, &doc.parsed.returns)
                {
                    let fix = (!expected.is_empty())
                        .then(|| {
                            self.rewrite_value_docstring_type_fix(function.body(), false, &expected)
                        })
                        .flatten();
                    self.push_with_fix(
                        "SKD203",
                        line,
                        format!(
                            "{prefix} return type(s) in docstring not consistent with the return annotation. {postfix}"
                        ),
                        suppression,
                        fix,
                    );
                }
            } else {
                self.push_generator_annotation_mismatch(line, &prefix, suppression);
            }
        } else if doc.parsed.has_returns_section
            && !self.options.check_return_types
            && generator_kind.is_none()
        {
            self.push_generator_annotation_mismatch(line, &prefix, suppression);
        }

        if !doc.parsed.has_yields_section {
            if !self.options.skip_checking_short_docstrings {
                let expected = return_expr.and_then(|expr| extract_yield_type(self.source, expr));
                let fix = ((generator_kind.is_some() || iterable) && expected.is_some())
                    .then(|| {
                        render_value_section(
                            self.options.style,
                            true,
                            expected.as_deref().unwrap_or_default(),
                        )
                    })
                    .and_then(|section| {
                        self.append_docstring_section_fix(function.body(), &section)
                    });
                self.push_with_fix(
                    "SKD402",
                    line,
                    format!("{prefix} has \"yield\" statements, but the docstring does not have a \"Yields\" section"),
                    suppression,
                    fix,
                );
            }
        } else if self.options.check_yield_types {
            if generator_kind.is_some() || iterable {
                let expected = return_expr.and_then(|expr| extract_yield_type(self.source, expr));
                if let Some(postfix) = yield_type_mismatch_message(
                    return_expr
                        .map(|expr| return_annotation_text(self.source, expr))
                        .as_deref(),
                    expected.as_deref(),
                    &doc.parsed.yields,
                    true,
                    self.options.require_yield_section_when_yielding_nothing,
                ) {
                    let fix = expected
                        .as_deref()
                        .filter(|ty| !ty.is_empty())
                        .and_then(|ty| {
                            self.rewrite_value_docstring_type_fix(function.body(), true, ty)
                        });
                    self.push_with_fix(
                        "SKD404",
                        line,
                        format!(
                            "{prefix} yield type(s) in docstring not consistent with the return annotation. {postfix}"
                        ),
                        suppression,
                        fix,
                    );
                }
            } else {
                self.push_generator_annotation_mismatch(line, &prefix, suppression);
            }
        } else if (generator_kind.is_none() || !iterable) && !facts.has_bare_return {
            // Mirrors pydoclint's historical condition exactly.
            self.push_generator_annotation_mismatch(line, &prefix, suppression);
        }
    }

    fn check_raises(
        &mut self,
        function: FunctionRef<'a>,
        parent: ParentDef<'a>,
        facts: &FunctionFacts,
        doc: &DocInfo,
    ) {
        let line = function.line(self.ast);
        let suppression = self.suppression_line(line, Some(doc.closing_line));
        let prefix = function_prefix(function, parent);
        if facts.has_raise && !doc.parsed.has_raises_section {
            let mut exceptions = facts.raised_exceptions.clone();
            if self
                .options
                .should_declare_assert_error_if_assert_statement_exists
                && facts.has_assert
            {
                exceptions.push("AssertionError".to_string());
            }
            exceptions.sort();
            exceptions.dedup();
            let fix = (!exceptions.is_empty())
                .then(|| render_raises_section(self.options.style, &exceptions))
                .and_then(|section| self.append_docstring_section_fix(function.body(), &section));
            self.push_with_fix(
                "SKD501",
                line,
                format!("{prefix} has raise statements, but the docstring does not have a \"Raises\" section"),
                suppression,
                fix,
            );
        }

        let no_runtime_exception_source = if self
            .options
            .should_declare_assert_error_if_assert_statement_exists
        {
            !facts.has_assert && !facts.has_raise
        } else {
            !facts.has_raise
        };
        if no_runtime_exception_source && doc.parsed.has_raises_section && !is_abstract(function) {
            self.push(
                "SKD502",
                line,
                format!("{prefix} has a \"Raises\" section in the docstring, but there are not \"raise\" statements in the body"),
                suppression,
            );
        }

        if self
            .options
            .should_declare_assert_error_if_assert_statement_exists
            && facts.has_assert
            && !doc.parsed.has_raises_section
        {
            let section =
                render_raises_section(self.options.style, &["AssertionError".to_string()]);
            let fix = self.append_docstring_section_fix(function.body(), &section);
            self.push_with_fix(
                "SKD504",
                line,
                format!("{prefix} has assert statements, but the docstring does not have a \"Raises\" section. (Assert statements could raise \"AssertError\".)"),
                suppression,
                fix,
            );
        }

        // Upstream only emits DOC503 when a real `raise` statement exists.
        if facts.has_raise {
            let mut actual = facts.raised_exceptions.clone();
            if self
                .options
                .should_declare_assert_error_if_assert_statement_exists
                && facts.has_assert
            {
                actual.push("AssertionError (implicitly from the `assert` statement)".to_string());
            }
            actual.sort();
            actual.dedup();
            let mut documented = doc.parsed.raises.clone();
            documented.sort();
            if !raises_match(&actual, &documented) {
                self.push(
                    "SKD503",
                    line,
                    format!(
                        "{prefix} exceptions in the \"Raises\" section in the docstring do not match those in the function body. Raised exceptions in the docstring: {}. Raised exceptions in the body: {}.",
                        python_list(&documented),
                        python_list(&actual)
                    ),
                    suppression,
                );
            }
        }
    }

    fn check_class(&mut self, class: &'a ast::StmtClassDef) {
        if !self.options.check_class_attributes {
            return;
        }
        let line = self.ast.location_of(class).line;
        let Some(doc) = self.class_doc_info(&class.body, line) else {
            return;
        };
        if doc.parsed.parse_error.is_some() {
            self.push(
                "SKD001",
                line,
                format!(
                    "Class `{}`: Potential formatting errors in docstring. Error message: {}",
                    class.name,
                    doc.parsed
                        .parse_error
                        .as_deref()
                        .unwrap_or("unknown parse error")
                ),
                self.suppression_line(line, Some(doc.closing_line)),
            );
            return;
        }
        if self.options.skip_checking_short_docstrings && doc.parsed.is_short {
            return;
        }

        let suppression = self.suppression_line(line, Some(doc.closing_line));
        let actual = self.actual_class_attributes(class);
        let mut documented = doc.parsed.attrs.clone();

        if self.options.require_inline_class_var_docs && !documented.is_empty() {
            self.push(
                "SKD607",
                line,
                format!(
                    "Class `{}`: The class docstring does not need an \"Attributes\" section, because the class attributes are documented inline.",
                    class.name
                ),
                suppression,
            );
            documented.clear();
        }

        self.apply_inline_attribute_docs(class, &actual, &mut documented, suppression);

        if documented.len() < actual.len() {
            let fix = (!doc.parsed.has_attributes_section
                && !self.options.require_inline_class_var_docs
                && !actual.is_empty())
            .then(|| render_parameter_section(self.options.style, &actual, true))
            .and_then(|section| self.append_docstring_section_fix(&class.body, &section));
            self.push_with_fix(
                "SKD601",
                line,
                format!(
                    "Class `{}`: Class docstring contains fewer class attributes than actual class attributes.  (Please read https://jsh9.github.io/pydoclint/checking_class_attributes.html on how to correctly document class attributes.)",
                    class.name
                ),
                suppression,
                fix,
            );
        }
        if documented.len() > actual.len() {
            self.push(
                "SKD602",
                line,
                format!(
                    "Class `{}`: Class docstring contains more class attributes than in actual class attributes.  (Please read https://jsh9.github.io/pydoclint/checking_class_attributes.html on how to correctly document class attributes.)",
                    class.name
                ),
                suppression,
            );
        }

        let actual_names = actual
            .iter()
            .map(|arg| arg.name.as_str())
            .collect::<Vec<_>>();
        let doc_names = documented
            .iter()
            .map(|arg| arg.name.as_str())
            .collect::<Vec<_>>();
        let same_unordered = same_string_set_with_equal_len(&actual_names, &doc_names);
        if !same_unordered {
            let mut missing = actual
                .iter()
                .filter(|arg| !documented.iter().any(|doc_arg| doc_arg.name == arg.name))
                .map(format_actual_arg)
                .collect::<Vec<_>>();
            let mut extra = documented
                .iter()
                .filter(|arg| !actual.iter().any(|actual_arg| actual_arg.name == arg.name))
                .map(format_doc_item)
                .collect::<Vec<_>>();
            missing.sort();
            extra.sort();
            let mut postfix = Vec::new();
            if !missing.is_empty() {
                let location = if self.options.require_inline_class_var_docs {
                    "not documented inline"
                } else {
                    "not in the docstring"
                };
                postfix.push(format!(
                    "Attributes in the class definition but {location}: [{}].",
                    missing.join(", ")
                ));
            }
            if !extra.is_empty() {
                postfix.push(format!(
                    "Arguments in the docstring but not in the actual class attributes: [{}].",
                    extra.join(", ")
                ));
            }
            self.push(
                "SKD603",
                line,
                format!(
                    "Class `{}`: Class docstring attributes are different from actual class attributes. (Or could be other formatting issues: https://jsh9.github.io/pydoclint/violation_codes.html#notes-on-doc103 ). {} (Please read https://jsh9.github.io/pydoclint/checking_class_attributes.html on how to correctly document class attributes.)",
                    class.name,
                    postfix.join(" ")
                ),
                suppression,
            );
            return;
        }
        if self.options.check_arg_order && actual_names != doc_names {
            let fix = self.reorder_docstring_items_fix(&class.body, true, &actual_names);
            self.push_with_fix(
                "SKD604",
                line,
                format!(
                    "Class `{}`: Attributes are the same in docstring and class def, but are in a different order.  (Please read https://jsh9.github.io/pydoclint/checking_class_attributes.html on how to correctly document class attributes.)",
                    class.name
                ),
                suppression,
                fix,
            );
        }

        if (self.options.arg_type_hints_in_signature && self.options.arg_type_hints_in_docstring)
            || (self.options.check_arg_order && actual_names != doc_names)
        {
            let mismatches = actual
                .iter()
                .filter_map(|arg| {
                    let doc_arg = documented.iter().find(|doc_arg| doc_arg.name == arg.name)?;
                    (!types_equal(&arg.ty, &doc_arg.ty)).then(|| arg.name.clone())
                })
                .collect::<Vec<_>>();
            if !mismatches.is_empty() {
                let desired = actual
                    .iter()
                    .filter(|arg| mismatches.contains(&arg.name) && !arg.ty.is_empty())
                    .map(|arg| (arg.name.clone(), arg.ty.clone()))
                    .collect::<Vec<_>>();
                let fix = (self.options.arg_type_hints_in_signature
                    && self.options.arg_type_hints_in_docstring
                    && desired.len() == mismatches.len())
                .then(|| self.rewrite_named_docstring_types_fix(&class.body, true, &desired))
                .flatten();
                self.push_with_fix(
                    "SKD605",
                    line,
                    format!(
                        "Class `{}`: Attribute names match, but type hints in these attributes do not match: {}  (Please read https://jsh9.github.io/pydoclint/checking_class_attributes.html on how to correctly document class attributes.)",
                        class.name,
                        mismatches.join(", ")
                    ),
                    suppression,
                    fix,
                );
            }
        }
    }

    fn actual_class_attributes(&self, class: &ast::StmtClassDef) -> Vec<ActualArg> {
        if !self
            .options
            .only_attrs_with_classvar_are_treated_as_class_attrs
        {
            if let Some(model_class) = self.dataclasses.class(class.name.as_str()) {
                if model_class.is_dataclass_like {
                    return self
                        .dataclasses
                        .effective_fields(class.name.as_str())
                        .into_iter()
                        .filter(|field| {
                            self.options.should_document_private_class_attributes
                                || !field.name.starts_with('_')
                        })
                        .map(|field| {
                            dataclass_field_to_actual(field, self.options.check_arg_defaults)
                        })
                        .collect();
                }
            }
        }

        let mut actual = Vec::new();
        for stmt in &class.body {
            match stmt {
                Stmt::AnnAssign(assign) => {
                    let target_name = canonical_expression_text(assign.target.as_ref());
                    let full = canonical_annotation_text(self.source, &assign.annotation);
                    let mut ty = if self
                        .options
                        .only_attrs_with_classvar_are_treated_as_class_attrs
                    {
                        let Some(inner) = classvar_inner_type(self.source, &assign.annotation)
                        else {
                            continue;
                        };
                        inner
                    } else {
                        full
                    };
                    if self.options.check_arg_defaults
                        && matches!(assign.target.as_ref(), Expr::Name(_))
                    {
                        if let Some(default) = assign.value.as_deref() {
                            append_default(&mut ty, &canonical_expression_text(default));
                        }
                    }
                    actual.push(ActualArg {
                        name: target_name,
                        ty,
                    });
                }
                Stmt::Assign(assign)
                    if !self
                        .options
                        .only_attrs_with_classvar_are_treated_as_class_attrs =>
                {
                    let default = self
                        .options
                        .check_arg_defaults
                        .then(|| canonical_expression_text(assign.value.as_ref()));
                    for target in &assign.targets {
                        let before = actual.len();
                        collect_assignment_names(self.source, target, &mut actual);
                        if let (Some(default), Expr::Name(_)) = (default.as_deref(), target) {
                            for arg in &mut actual[before..] {
                                append_default(&mut arg.ty, default);
                            }
                        }
                    }
                }
                Stmt::FunctionDef(function)
                    if self.options.treat_property_methods_as_class_attributes
                        && function
                            .decorator_list
                            .first()
                            .is_some_and(is_property_decorator) =>
                {
                    actual.push(ActualArg {
                        name: function.name.to_string(),
                        ty: function
                            .returns
                            .as_deref()
                            .map(|expr| canonical_annotation_text(self.source, expr))
                            .unwrap_or_default(),
                    });
                }
                Stmt::AsyncFunctionDef(function)
                    if self.options.treat_property_methods_as_class_attributes
                        && function
                            .decorator_list
                            .first()
                            .is_some_and(is_property_decorator) =>
                {
                    actual.push(ActualArg {
                        name: function.name.to_string(),
                        ty: function
                            .returns
                            .as_deref()
                            .map(|expr| canonical_annotation_text(self.source, expr))
                            .unwrap_or_default(),
                    });
                }
                _ => {}
            }
        }
        if !self.options.should_document_private_class_attributes {
            actual.retain(|arg| !arg.name.starts_with('_'));
        }
        actual
    }

    fn apply_inline_attribute_docs(
        &mut self,
        class: &ast::StmtClassDef,
        actual: &[ActualArg],
        documented: &mut Vec<DocItem>,
        suppression: usize,
    ) {
        for pair in class.body.windows(2) {
            let Some(name) = assignment_single_name(self.source, &pair[0]) else {
                continue;
            };
            if !actual.iter().any(|arg| arg.name == name) {
                continue;
            }
            let Stmt::Expr(doc_stmt) = &pair[1] else {
                continue;
            };
            let Some(text) = string_constant(&doc_stmt.value) else {
                continue;
            };
            if !self.options.require_inline_class_var_docs {
                let line = self.ast.location_of(doc_stmt).line;
                self.push(
                    "SKD606",
                    line,
                    format!(
                        "Class `{}`, Attribute `{name}`: This attribute is documented inline, but it should be documented in the class docstring instead.",
                        class.name
                    ),
                    suppression,
                );
                continue;
            }
            let ty = if self.options.arg_type_hints_in_docstring {
                text.lines()
                    .next()
                    .and_then(|line| line.split_once(':').map(|(ty, _)| ty.trim().to_string()))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            if let Some(existing) = documented.iter_mut().find(|item| item.name == name) {
                existing.ty = ty;
            } else {
                let description = text
                    .lines()
                    .next()
                    .and_then(|line| {
                        line.split_once(':')
                            .map(|(_, description)| description.trim())
                    })
                    .unwrap_or_default()
                    .to_string();
                documented.push(DocItem {
                    name,
                    ty,
                    description,
                });
            }
        }

        if self.options.require_inline_class_var_docs {
            documented.sort_by_key(|doc| {
                actual
                    .iter()
                    .position(|arg| arg.name == doc.name)
                    .unwrap_or(usize::MAX)
            });
        }
    }

    fn constructor_doc(
        &mut self,
        function: FunctionRef<'a>,
        parent: ParentDef<'a>,
        init_doc: Option<DocInfo>,
    ) -> Option<DocInfo> {
        let ParentDef::Class(class) = parent else {
            return init_doc;
        };
        let class_name = class.name.as_str();
        let class_line = self.ast.location_of(class).line;
        let class_doc = self.class_doc_info(&class.body, class_line);
        let class_suppression = self.owner_suppression_line(&class.body, class_line);
        let line = function.line(self.ast);

        let Some(init_doc) = init_doc else {
            return class_doc;
        };
        if !self.options.allow_init_docstring {
            self.push(
                "SKD301",
                line,
                format!(
                    "Class `{class_name}`: __init__() should not have a docstring; please combine it with the docstring of the class"
                ),
                self.suppression_line(line, Some(init_doc.closing_line)),
            );
            return class_doc;
        }

        let init_suppression = self.suppression_line(line, Some(init_doc.closing_line));
        if let Some(class_doc) = &class_doc {
            if let Some(error) = class_doc.parsed.parse_error.as_deref() {
                let already_reported = self
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "SKD001" && diagnostic.line == class_line);
                if !already_reported {
                    self.push(
                        "SKD001",
                        class_line,
                        format!(
                            "Class `{class_name}`: Potential formatting errors in docstring. Error message: {error}"
                        ),
                        class_suppression,
                    );
                }
                // The class-side structure is unreliable. Continue with the
                // separate __init__ docstring, but do not emit class structural
                // diagnostics from a broken parse.
                return Some(init_doc);
            }

            // Preserve pydoclint 0.9.1 emission order exactly. This ordering is
            // observable in baseline files: 302 -> 303 -> 304 -> 306 -> 307 -> 305.
            if class_doc.parsed.has_returns_section {
                self.push("SKD302", class_line, format!("Class `{class_name}`: The class docstring does not need a \"Returns\" section, because __init__() cannot return anything"), class_suppression);
            }
            if init_doc.parsed.has_returns_section {
                self.push("SKD303", line, format!("Class `{class_name}`: The __init__() docstring does not need a \"Returns\" section, because it cannot return anything"), init_suppression);
            }
            if class_doc.parsed.has_args_section && !class_doc.parsed.params.is_empty() {
                self.push("SKD304", class_line, format!("Class `{class_name}`: Class docstring has an argument/parameter section; please put it in the __init__() docstring"), class_suppression);
            }
            if class_doc.parsed.has_yields_section {
                self.push("SKD306", class_line, format!("Class `{class_name}`: The class docstring does not need a \"Yields\" section, because __init__() cannot yield anything"), class_suppression);
            }
            if init_doc.parsed.has_yields_section {
                self.push("SKD307", class_line, format!("Class `{class_name}`: The __init__() docstring does not need a \"Yields\" section, because __init__() cannot yield anything"), class_suppression);
            }
            if class_doc.parsed.has_raises_section {
                self.push("SKD305", class_line, format!("Class `{class_name}`: Class docstring has a \"Raises\" section; please put it in the __init__() docstring"), class_suppression);
            }
        } else {
            if init_doc.parsed.has_returns_section {
                self.push("SKD303", line, format!("Class `{class_name}`: The __init__() docstring does not need a \"Returns\" section, because it cannot return anything"), init_suppression);
            }
            if init_doc.parsed.has_yields_section {
                self.push("SKD307", class_line, format!("Class `{class_name}`: The __init__() docstring does not need a \"Yields\" section, because __init__() cannot yield anything"), class_suppression);
            }
        }
        Some(init_doc)
    }

    fn check_constructor_return_sections(&mut self, parent: ParentDef<'a>, doc: &DocInfo) {
        let ParentDef::Class(class) = parent else {
            return;
        };
        let class_name = class.name.as_str();
        let line = self.ast.location_of(class).line;
        let suppression = self.owner_suppression_line(&class.body, line);
        if doc.parsed.has_returns_section {
            self.push("SKD302", line, format!("Class `{class_name}`: The class docstring does not need a \"Returns\" section, because __init__() cannot return anything"), suppression);
        }
        if doc.parsed.has_yields_section {
            self.push("SKD306", line, format!("Class `{class_name}`: The class docstring does not need a \"Yields\" section, because __init__() cannot yield anything"), suppression);
        }
    }

    fn is_last_constructor(&self, function: FunctionRef<'a>, parent: ParentDef<'a>) -> bool {
        let ParentDef::Class(class) = parent else {
            return true;
        };
        let current_line = function.line(self.ast);
        !class.body.iter().any(|stmt| match stmt {
            Stmt::FunctionDef(node) => {
                node.name.as_str() == "__init__" && self.ast.location_of(node).line > current_line
            }
            Stmt::AsyncFunctionDef(node) => {
                node.name.as_str() == "__init__" && self.ast.location_of(node).line > current_line
            }
            _ => false,
        })
    }

    fn doc_info(&self, body: &'a [Stmt], definition_line: usize) -> Option<DocInfo> {
        let text = suite_docstring(body)?;
        let stmt = suite_docstring_stmt(body)?;
        let (parsed, style_mismatch) = if self.options.check_style_mismatch {
            parse_docstring_with_style_detection(text, self.options.style)
        } else {
            (parse_docstring(text, self.options.style), false)
        };
        let closing_line = self.ast.end_location_of(stmt).line.max(definition_line);
        Some(DocInfo {
            parsed,
            closing_line,
            style_mismatch,
        })
    }

    fn class_doc_info(&self, body: &'a [Stmt], definition_line: usize) -> Option<DocInfo> {
        let text = suite_docstring(body)?;
        let stmt = suite_docstring_stmt(body)?;
        let parsed = parse_docstring(text, self.options.style);
        let closing_line = self.ast.end_location_of(stmt).line.max(definition_line);
        Some(DocInfo {
            parsed,
            closing_line,
            style_mismatch: false,
        })
    }

    fn owner_suppression_line(&self, body: &'a [Stmt], definition_line: usize) -> usize {
        match self.options.native_mode_noqa_location {
            NoqaLocation::Definition => definition_line,
            NoqaLocation::Docstring => suite_docstring_stmt(body)
                .map(|stmt| self.ast.end_location_of(stmt).line.max(definition_line))
                // Upstream has no suppression mapping when docstring mode is
                // selected but this owner has no docstring. Line 0 cannot
                // match a real Python comment and preserves that behavior.
                .unwrap_or(0),
        }
    }

    fn suppression_line(&self, definition_line: usize, docstring_line: Option<usize>) -> usize {
        match self.options.native_mode_noqa_location {
            NoqaLocation::Definition => definition_line,
            NoqaLocation::Docstring => docstring_line.unwrap_or(definition_line),
        }
    }

    fn push(&mut self, code: &str, line: usize, message: String, suppression_line: usize) {
        self.push_with_fix(code, line, message, suppression_line, None);
    }

    fn push_with_fix(
        &mut self,
        code: &str,
        line: usize,
        message: String,
        suppression_line: usize,
        fix: Option<Fix>,
    ) {
        if !self.config.is_enabled(code) {
            return;
        }
        let mut diagnostic = Diagnostic::new(
            code,
            message,
            self.display_path.clone(),
            Span::new(line, 1, line, 1),
            "warning",
        );
        diagnostic.suppression_line = Some(suppression_line);
        if let Some(fix) = fix {
            diagnostic.fix = Some(fix);
        }
        self.diagnostics.push(diagnostic);
    }

    fn append_docstring_section_fix(&self, body: &'a [Stmt], section: &str) -> Option<Fix> {
        if !self.allow_structural_fixes {
            return None;
        }
        let stmt = suite_docstring_stmt(body)?;
        let literal = source_text(self.source, stmt);
        if !literal.contains('\n') {
            return None;
        }

        let double = literal.rfind("\"\"\"");
        let single = literal.rfind("'''");
        let close = match (double, single) {
            (Some(left), Some(right)) => left.max(right),
            (Some(index), None) | (None, Some(index)) => index,
            (None, None) => return None,
        };
        let before_close = &literal[..close];
        let last_newline = before_close.rfind('\n')?;
        let closing_indent = &before_close[last_newline + 1..];
        if !closing_indent.chars().all(|ch| matches!(ch, ' ' | '\t')) {
            return None;
        }

        let newline = if literal.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let content = &before_close[..last_newline + 1];
        let mut rendered = String::new();
        for (index, line) in section.lines().enumerate() {
            if index > 0 {
                rendered.push_str(newline);
            }
            rendered.push_str(closing_indent);
            rendered.push_str(line);
        }

        let mut replacement = String::with_capacity(literal.len() + rendered.len() + 4);
        replacement.push_str(content);
        if !content.ends_with(&format!("{newline}{newline}")) {
            replacement.push_str(newline);
        }
        replacement.push_str(&rendered);
        replacement.push_str(newline);
        replacement.push_str(closing_indent);
        replacement.push_str(&literal[close..]);

        let start = self.ast.location_of(stmt);
        let end = self.ast.end_location_of(stmt);
        Some(Fix {
            safe: true,
            message: "Add missing docstring section".to_string(),
            replacement,
            start_line: start.line,
            start_column: start.column,
            end_line: end.line,
            end_column: end.column,
        })
    }

    fn reorder_docstring_items_fix(
        &self,
        body: &'a [Stmt],
        attributes: bool,
        desired_names: &[&str],
    ) -> Option<Fix> {
        if !self.allow_structural_fixes {
            return None;
        }
        if !matches!(self.options.style, DocStyle::Google | DocStyle::Numpy) {
            return None;
        }
        let stmt = suite_docstring_stmt(body)?;
        let literal = source_text(self.source, stmt);
        let replacement = reorder_docstring_literal_items(
            literal,
            self.options.style,
            attributes,
            desired_names,
        )?;
        if replacement == literal {
            return None;
        }
        let start = self.ast.location_of(stmt);
        let end = self.ast.end_location_of(stmt);
        Some(Fix {
            safe: true,
            message: if attributes {
                "Reorder docstring attribute blocks to match the class".to_string()
            } else {
                "Reorder docstring argument blocks to match the signature".to_string()
            },
            replacement,
            start_line: start.line,
            start_column: start.column,
            end_line: end.line,
            end_column: end.column,
        })
    }

    fn rewrite_named_docstring_types_fix(
        &self,
        body: &'a [Stmt],
        attributes: bool,
        desired: &[(String, String)],
    ) -> Option<Fix> {
        if !self.allow_structural_fixes || desired.is_empty() {
            return None;
        }
        let stmt = suite_docstring_stmt(body)?;
        let literal = source_text(self.source, stmt);
        let replacement =
            rewrite_named_doc_types_literal(literal, self.options.style, attributes, desired)?;
        if replacement == literal {
            return None;
        }
        let start = self.ast.location_of(stmt);
        let end = self.ast.end_location_of(stmt);
        Some(Fix {
            safe: true,
            message: if attributes {
                "Synchronize docstring attribute types with class annotations".to_string()
            } else {
                "Synchronize docstring argument types with signature annotations".to_string()
            },
            replacement,
            start_line: start.line,
            start_column: start.column,
            end_line: end.line,
            end_column: end.column,
        })
    }

    fn rewrite_value_docstring_type_fix(
        &self,
        body: &'a [Stmt],
        yields: bool,
        desired_type: &str,
    ) -> Option<Fix> {
        if !self.allow_structural_fixes || desired_type.trim().is_empty() {
            return None;
        }
        let stmt = suite_docstring_stmt(body)?;
        let literal = source_text(self.source, stmt);
        let replacement =
            rewrite_value_doc_type_literal(literal, self.options.style, yields, desired_type)?;
        if replacement == literal {
            return None;
        }
        let start = self.ast.location_of(stmt);
        let end = self.ast.end_location_of(stmt);
        Some(Fix {
            safe: true,
            message: if yields {
                "Synchronize docstring yield type with the return annotation".to_string()
            } else {
                "Synchronize docstring return type with the return annotation".to_string()
            },
            replacement,
            start_line: start.line,
            start_column: start.column,
            end_line: end.line,
            end_column: end.column,
        })
    }

    fn push_generator_annotation_mismatch(
        &mut self,
        line: usize,
        prefix: &str,
        suppression: usize,
    ) {
        self.push(
            "SKD405",
            line,
            format!(
                "{prefix} has both \"return\" and \"yield\" statements. Please use Generator[YieldType, SendType, ReturnType] as the return type annotation, and put your yield type in YieldType and return type in ReturnType. More details in https://jsh9.github.io/pydoclint/notes_generator_vs_iterator.html"
            ),
            suppression,
        );
    }
}

fn reorder_docstring_literal_items(
    literal: &str,
    style: DocStyle,
    attributes: bool,
    desired_names: &[&str],
) -> Option<String> {
    if desired_names.is_empty() {
        return None;
    }
    let mut unique = std::collections::HashSet::new();
    if !desired_names.iter().all(|name| unique.insert(*name)) {
        return None;
    }

    let newline = if literal.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let normalized = if newline == "\r\n" {
        literal.replace("\r\n", "\n")
    } else {
        literal.to_string()
    };
    let mut lines = normalized
        .split('\n')
        .map(str::to_string)
        .collect::<Vec<_>>();
    let close_line = lines.iter().rposition(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''")
    })?;
    let base_indent = lines[close_line]
        .chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .count();

    let header_index = match style {
        DocStyle::Google => {
            let headings: &[&str] = if attributes {
                &["Attributes:"]
            } else {
                &["Args:", "Arguments:", "Parameters:", "Parameter:"]
            };
            lines.iter().enumerate().find_map(|(index, line)| {
                (line_indent(line) == base_indent && headings.contains(&line.trim()))
                    .then_some(index)
            })?
        }
        DocStyle::Numpy => {
            let headings: &[&str] = if attributes {
                &["Attributes"]
            } else {
                &["Parameters", "Parameter", "Args", "Arguments"]
            };
            lines.iter().enumerate().find_map(|(index, line)| {
                let is_heading =
                    line_indent(line) == base_indent && headings.contains(&line.trim());
                let underlined = lines
                    .get(index + 1)
                    .is_some_and(|next| is_numpy_rule_line(next.trim()));
                (is_heading && underlined).then_some(index)
            })?
        }
        DocStyle::Sphinx => return None,
    };

    let start = match style {
        DocStyle::Google => header_index + 1,
        DocStyle::Numpy => header_index + 2,
        DocStyle::Sphinx => unreachable!(),
    };
    let mut end = close_line;
    match style {
        DocStyle::Google => {
            for (index, line) in lines.iter().enumerate().take(close_line).skip(start) {
                if !line.trim().is_empty() && line_indent(line) <= base_indent {
                    end = index;
                    break;
                }
            }
        }
        DocStyle::Numpy => {
            for index in start..close_line {
                if line_indent(&lines[index]) == base_indent
                    && !lines[index].trim().is_empty()
                    && lines
                        .get(index + 1)
                        .is_some_and(|next| is_numpy_rule_line(next.trim()))
                {
                    end = index;
                    break;
                }
            }
        }
        DocStyle::Sphinx => unreachable!(),
    }

    let mut content_end = end;
    while content_end > start && lines[content_end - 1].trim().is_empty() {
        content_end -= 1;
    }
    if content_end <= start {
        return None;
    }

    let mut candidates = Vec::<(usize, &str, usize)>::new();
    for (index, line) in lines.iter().enumerate().take(content_end).skip(start) {
        if let Some(name) = desired_names
            .iter()
            .copied()
            .find(|name| line_declares_doc_item(line, name, style))
        {
            candidates.push((index, name, line_indent(line)));
        }
    }
    if candidates.is_empty() {
        return None;
    }
    let declaration_indent = candidates.iter().map(|(_, _, indent)| *indent).min()?;
    candidates.retain(|(_, _, indent)| *indent == declaration_indent);
    if candidates.len() != desired_names.len() {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    if !candidates.iter().all(|(_, name, _)| seen.insert(*name)) {
        return None;
    }

    let mut blocks = std::collections::HashMap::<&str, Vec<String>>::new();
    for (position, (block_start, name, _)) in candidates.iter().enumerate() {
        let block_end = candidates
            .get(position + 1)
            .map(|(index, _, _)| *index)
            .unwrap_or(content_end);
        blocks.insert(*name, lines[*block_start..block_end].to_vec());
    }
    if desired_names.iter().any(|name| !blocks.contains_key(name)) {
        return None;
    }

    let mut reordered = Vec::new();
    for name in desired_names {
        reordered.extend(blocks.remove(name)?);
    }
    lines.splice(start..content_end, reordered);
    let output = lines.join("\n");
    Some(if newline == "\r\n" {
        output.replace('\n', "\r\n")
    } else {
        output
    })
}

fn rewrite_named_doc_types_literal(
    literal: &str,
    style: DocStyle,
    attributes: bool,
    desired: &[(String, String)],
) -> Option<String> {
    let newline = if literal.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let normalized = if newline == "\r\n" {
        literal.replace("\r\n", "\n")
    } else {
        literal.to_string()
    };
    let mut lines = normalized
        .split('\n')
        .map(str::to_string)
        .collect::<Vec<_>>();
    let close_line = lines.iter().rposition(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''")
    })?;
    let base_indent = line_indent(&lines[close_line]);
    let targets = desired.iter().cloned().collect::<HashMap<_, _>>();
    let mut touched = std::collections::HashSet::<String>::new();

    match style {
        DocStyle::Google => {
            let headings: &[&str] = if attributes {
                &["Attributes:"]
            } else {
                &["Args:", "Arguments:", "Parameters:", "Params:"]
            };
            let header = lines.iter().enumerate().find_map(|(index, line)| {
                (line_indent(line) == base_indent && headings.contains(&line.trim()))
                    .then_some(index)
            })?;
            let end = (header + 1..close_line)
                .find(|index| {
                    !lines[*index].trim().is_empty() && line_indent(&lines[*index]) <= base_indent
                })
                .unwrap_or(close_line);
            let item_indent = (header + 1..end)
                .filter(|index| !lines[*index].trim().is_empty())
                .map(|index| line_indent(&lines[index]))
                .filter(|indent| *indent > base_indent)
                .min()?;
            for line in lines.iter_mut().take(end).skip(header + 1) {
                if line.trim().is_empty() || line_indent(line) != item_indent {
                    continue;
                }
                let trimmed = line.trim();
                let Some(colon) = find_top_level_colon(trimmed) else {
                    continue;
                };
                let head = trimmed[..colon].trim();
                let (name, _) = split_google_name_and_type(head);
                let Some(new_type) = targets.get(&name) else {
                    continue;
                };
                let indent_bytes = line.len() - line.trim_start().len();
                let indent = line[..indent_bytes].to_string();
                let suffix = &trimmed[colon..];
                let new_head = if new_type.trim().is_empty() {
                    name.clone()
                } else {
                    format!("{name} ({})", new_type.trim())
                };
                *line = format!("{indent}{new_head}{suffix}");
                touched.insert(name);
            }
        }
        DocStyle::Numpy => {
            let headings: &[&str] = if attributes {
                &["Attributes", "Attribute"]
            } else {
                &["Parameters", "Parameter", "Params", "Arguments", "Args"]
            };
            let header = lines.iter().enumerate().find_map(|(index, line)| {
                let heading = line_indent(line) == base_indent && headings.contains(&line.trim());
                let underline = lines
                    .get(index + 1)
                    .is_some_and(|next| is_numpy_rule_line(next.trim()));
                (heading && underline).then_some(index)
            })?;
            let start = header + 2;
            let end = (start..close_line)
                .find(|index| {
                    line_indent(&lines[*index]) == base_indent
                        && !lines[*index].trim().is_empty()
                        && lines
                            .get(*index + 1)
                            .is_some_and(|next| is_numpy_rule_line(next.trim()))
                })
                .unwrap_or(close_line);
            for line in lines.iter_mut().take(end).skip(start) {
                if line.trim().is_empty() || line_indent(line) != base_indent {
                    continue;
                }
                let trimmed = line.trim();
                let raw_name = find_top_level_colon(trimmed)
                    .map(|colon| trimmed[..colon].trim())
                    .unwrap_or(trimmed);
                let name = normalize_doc_name(raw_name);
                let Some(new_type) = targets.get(&name) else {
                    continue;
                };
                let indent_bytes = line.len() - line.trim_start().len();
                let indent = line[..indent_bytes].to_string();
                *line = if new_type.trim().is_empty() {
                    format!("{indent}{raw_name}")
                } else {
                    format!("{indent}{raw_name} : {}", new_type.trim())
                };
                touched.insert(name);
            }
        }
        DocStyle::Sphinx if attributes => {
            for (target_name, new_type) in desired {
                let close_line = lines.iter().rposition(|line| {
                    let trimmed = line.trim_start();
                    trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''")
                })?;
                let mut directive_index = None;
                for (index, line) in lines.iter().enumerate().take(close_line) {
                    if line_indent(line) != base_indent {
                        continue;
                    }
                    let Some(name) =
                        sphinx_attribute_directive_name(line.trim()).map(normalize_doc_name)
                    else {
                        continue;
                    };
                    if name == *target_name {
                        directive_index = Some(index);
                        break;
                    }
                }
                let Some(index) = directive_index else {
                    continue;
                };
                let directive_indent = line_indent(&lines[index]);
                let close_line = lines.iter().rposition(|line| {
                    let trimmed = line.trim_start();
                    trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''")
                })?;
                let mut end = index + 1;
                while end < close_line {
                    if !lines[end].trim().is_empty() && line_indent(&lines[end]) <= directive_indent
                    {
                        break;
                    }
                    end += 1;
                }
                let type_line =
                    (index + 1..end).find(|inner| lines[*inner].trim().starts_with(":type:"));
                if let Some(type_line) = type_line {
                    if new_type.trim().is_empty() {
                        lines.remove(type_line);
                    } else {
                        let indent_bytes =
                            lines[type_line].len() - lines[type_line].trim_start().len();
                        let indent = lines[type_line][..indent_bytes].to_string();
                        lines[type_line] = format!("{indent}:type: {}", new_type.trim());
                    }
                } else if !new_type.trim().is_empty() {
                    let prefix_bytes = lines[index].len() - lines[index].trim_start().len();
                    let prefix = lines[index][..prefix_bytes].to_string();
                    lines.insert(index + 1, format!("{prefix}    :type: {}", new_type.trim()));
                }
                touched.insert(target_name.clone());
            }
        }
        DocStyle::Sphinx => {
            for (target_name, new_type) in desired {
                let close_line = lines.iter().rposition(|line| {
                    let trimmed = line.trim_start();
                    trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''")
                })?;
                let mut param_index = None;
                let mut type_index = None;
                for (index, line) in lines.iter().enumerate().take(close_line) {
                    if line_indent(line) != base_indent {
                        continue;
                    }
                    let Some(field) = parse_sphinx_field(line.trim()) else {
                        continue;
                    };
                    if is_sphinx_param_key(&field.key) {
                        let name = match field.args.as_slice() {
                            [name] => normalize_doc_name(name),
                            [_, name] => normalize_doc_name(name),
                            _ => continue,
                        };
                        if name == *target_name {
                            param_index = Some(index);
                        }
                    } else if field.key == "type"
                        && field.args.len() == 1
                        && normalize_doc_name(&field.args[0]) == *target_name
                    {
                        type_index = Some(index);
                    }
                }
                let Some(param_index) = param_index else {
                    continue;
                };
                let field = parse_sphinx_field(lines[param_index].trim())?;
                let indent_bytes = lines[param_index].len() - lines[param_index].trim_start().len();
                let indent = lines[param_index][..indent_bytes].to_string();
                if field.args.len() == 2 {
                    let suffix = field.description.as_str();
                    let head = if new_type.trim().is_empty() {
                        format!(":{} {target_name}:", field.key)
                    } else {
                        format!(":{} {} {target_name}:", field.key, new_type.trim())
                    };
                    lines[param_index] = if suffix.is_empty() {
                        format!("{indent}{head}")
                    } else {
                        format!("{indent}{head} {suffix}")
                    };
                    if let Some(type_index) = type_index {
                        lines.remove(type_index);
                    }
                } else if let Some(type_index) = type_index {
                    if new_type.trim().is_empty() {
                        lines.remove(type_index);
                    } else {
                        let type_indent_bytes =
                            lines[type_index].len() - lines[type_index].trim_start().len();
                        let type_indent = lines[type_index][..type_indent_bytes].to_string();
                        lines[type_index] =
                            format!("{type_indent}:type {target_name}: {}", new_type.trim());
                    }
                } else if !new_type.trim().is_empty() {
                    lines.insert(
                        param_index + 1,
                        format!("{indent}:type {target_name}: {}", new_type.trim()),
                    );
                }
                touched.insert(target_name.clone());
            }
        }
    }

    if targets.keys().any(|name| !touched.contains(name)) {
        return None;
    }
    let output = lines.join("\n");
    Some(if newline == "\r\n" {
        output.replace('\n', "\r\n")
    } else {
        output
    })
}

fn rewrite_value_doc_type_literal(
    literal: &str,
    style: DocStyle,
    yields: bool,
    desired_type: &str,
) -> Option<String> {
    let newline = if literal.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let normalized = if newline == "\r\n" {
        literal.replace("\r\n", "\n")
    } else {
        literal.to_string()
    };
    let mut lines = normalized
        .split('\n')
        .map(str::to_string)
        .collect::<Vec<_>>();
    let close_line = lines.iter().rposition(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''")
    })?;
    let base_indent = line_indent(&lines[close_line]);

    match style {
        DocStyle::Google => {
            let heading = if yields { "Yields:" } else { "Returns:" };
            let header = lines.iter().enumerate().find_map(|(index, line)| {
                (line_indent(line) == base_indent && line.trim() == heading).then_some(index)
            })?;
            let end = (header + 1..close_line)
                .find(|index| {
                    !lines[*index].trim().is_empty() && line_indent(&lines[*index]) <= base_indent
                })
                .unwrap_or(close_line);
            let item_indent = (header + 1..end)
                .filter(|index| !lines[*index].trim().is_empty())
                .map(|index| line_indent(&lines[index]))
                .filter(|indent| *indent > base_indent)
                .min()?;
            let index = (header + 1..end).find(|index| {
                !lines[*index].trim().is_empty() && line_indent(&lines[*index]) == item_indent
            })?;
            let trimmed = lines[index].trim();
            let colon = find_top_level_colon(trimmed)?;
            let indent_bytes = lines[index].len() - lines[index].trim_start().len();
            let indent = lines[index][..indent_bytes].to_string();
            lines[index] = format!("{indent}{}{}", desired_type.trim(), &trimmed[colon..]);
        }
        DocStyle::Numpy => {
            let heading = if yields { "Yields" } else { "Returns" };
            let header = lines.iter().enumerate().find_map(|(index, line)| {
                let heading_match = line_indent(line) == base_indent && line.trim() == heading;
                let underline = lines
                    .get(index + 1)
                    .is_some_and(|next| is_numpy_rule_line(next.trim()));
                (heading_match && underline).then_some(index)
            })?;
            let start = header + 2;
            let end = (start..close_line)
                .find(|index| {
                    line_indent(&lines[*index]) == base_indent
                        && !lines[*index].trim().is_empty()
                        && lines
                            .get(*index + 1)
                            .is_some_and(|next| is_numpy_rule_line(next.trim()))
                })
                .unwrap_or(close_line);
            let declarations = (start..end)
                .filter(|index| {
                    !lines[*index].trim().is_empty() && line_indent(&lines[*index]) == base_indent
                })
                .collect::<Vec<_>>();
            if declarations.len() != 1 {
                return None;
            }
            let index = declarations[0];
            let indent_bytes = lines[index].len() - lines[index].trim_start().len();
            let indent = lines[index][..indent_bytes].to_string();
            lines[index] = format!("{indent}{}", desired_type.trim());
        }
        DocStyle::Sphinx => {
            let type_key = if yields { "ytype" } else { "rtype" };
            let value_keys: &[&str] = if yields {
                &["yield", "yields"]
            } else {
                &["return", "returns"]
            };
            let mut value_line = None;
            let mut type_line = None;
            for (index, line) in lines.iter().enumerate().take(close_line) {
                if line_indent(line) != base_indent {
                    continue;
                }
                let Some(field) = parse_sphinx_field(line.trim()) else {
                    continue;
                };
                if field.key == type_key && field.args.is_empty() {
                    type_line = Some(index);
                } else if value_keys.contains(&field.key.as_str()) {
                    value_line = Some(index);
                }
            }
            if let Some(index) = type_line {
                let indent_bytes = lines[index].len() - lines[index].trim_start().len();
                let indent = lines[index][..indent_bytes].to_string();
                lines[index] = format!("{indent}:{type_key}: {}", desired_type.trim());
            } else {
                let index = value_line?;
                let field = parse_sphinx_field(lines[index].trim())?;
                let indent_bytes = lines[index].len() - lines[index].trim_start().len();
                let indent = lines[index][..indent_bytes].to_string();
                if field.args.len() == 1 {
                    let suffix = field.description.as_str();
                    let head = format!(":{} {}:", field.key, desired_type.trim());
                    lines[index] = if suffix.is_empty() {
                        format!("{indent}{head}")
                    } else {
                        format!("{indent}{head} {suffix}")
                    };
                } else {
                    lines.insert(
                        index + 1,
                        format!("{indent}:{type_key}: {}", desired_type.trim()),
                    );
                }
            }
        }
    }

    let output = lines.join("\n");
    Some(if newline == "\r\n" {
        output.replace('\n', "\r\n")
    } else {
        output
    })
}

fn line_indent(line: &str) -> usize {
    line.chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .count()
}

fn is_numpy_rule_line(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|ch| ch == '-')
}

fn line_declares_doc_item(line: &str, name: &str, style: DocStyle) -> bool {
    let trimmed = line.trim();
    let Some(rest) = trimmed.strip_prefix(name) else {
        return false;
    };
    if rest.is_empty() {
        return style == DocStyle::Numpy;
    }
    match style {
        DocStyle::Google => {
            rest.starts_with(':')
                || rest
                    .trim_start()
                    .strip_prefix('(')
                    .is_some_and(|tail| tail.contains("):"))
        }
        DocStyle::Numpy => rest.trim_start().starts_with(':'),
        DocStyle::Sphinx => false,
    }
}

fn render_parameter_section(style: DocStyle, items: &[ActualArg], attributes: bool) -> String {
    match style {
        DocStyle::Google => {
            let heading = if attributes { "Attributes:" } else { "Args:" };
            let mut lines = vec![heading.to_string()];
            for item in items {
                let type_suffix = if item.ty.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", item.ty)
                };
                lines.push(format!(
                    "    {}{}: Value for `{}`.",
                    item.name, type_suffix, item.name
                ));
            }
            lines.join("\n")
        }
        DocStyle::Numpy => {
            let heading = if attributes {
                "Attributes"
            } else {
                "Parameters"
            };
            let mut lines = vec![heading.to_string(), "-".repeat(heading.len())];
            for item in items {
                if item.ty.is_empty() {
                    lines.push(item.name.clone());
                } else {
                    lines.push(format!("{} : {}", item.name, item.ty));
                }
                lines.push(format!("    Value for `{}`.", item.name));
            }
            lines.join("\n")
        }
        DocStyle::Sphinx if attributes => {
            let mut blocks = Vec::new();
            for item in items {
                let mut lines = vec![format!(".. attribute :: {}", item.name)];
                if !item.ty.is_empty() {
                    lines.push(format!("    :type: {}", item.ty));
                }
                lines.push(format!("    Value for `{}`.", item.name));
                blocks.push(lines.join("\n"));
            }
            blocks.join("\n\n")
        }
        DocStyle::Sphinx => {
            let mut lines = Vec::new();
            for item in items {
                lines.push(format!(":param {}: Value for `{}`.", item.name, item.name));
                if !item.ty.is_empty() {
                    lines.push(format!(":type {}: {}", item.name, item.ty));
                }
            }
            lines.join("\n")
        }
    }
}

fn render_value_section(style: DocStyle, yields: bool, ty: &str) -> String {
    let (google_heading, numpy_heading, sphinx_value, sphinx_type, description) = if yields {
        ("Yields:", "Yields", ":yield:", ":ytype:", "Yielded value.")
    } else {
        (
            "Returns:",
            "Returns",
            ":return:",
            ":rtype:",
            "Return value.",
        )
    };
    match style {
        DocStyle::Google => format!("{google_heading}\n    {ty}: {description}"),
        DocStyle::Numpy => format!(
            "{numpy_heading}\n{}\n{ty}\n    {description}",
            "-".repeat(numpy_heading.len())
        ),
        DocStyle::Sphinx => format!("{sphinx_value} {description}\n{sphinx_type} {ty}"),
    }
}

fn render_raises_section(style: DocStyle, exceptions: &[String]) -> String {
    match style {
        DocStyle::Google => {
            let mut lines = vec!["Raises:".to_string()];
            for exception in exceptions {
                lines.push(format!(
                    "    {exception}: Raised when this condition occurs."
                ));
            }
            lines.join("\n")
        }
        DocStyle::Numpy => {
            let mut lines = vec!["Raises".to_string(), "------".to_string()];
            for exception in exceptions {
                lines.push(exception.clone());
                lines.push("    Raised when this condition occurs.".to_string());
            }
            lines.join("\n")
        }
        DocStyle::Sphinx => exceptions
            .iter()
            .map(|exception| format!(":raises {exception}: Raised when this condition occurs."))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn collect_function_args(
    source: &str,
    function: FunctionRef<'_>,
    parent: ParentDef<'_>,
    options: &PydoclintOptions,
) -> Vec<ActualArg> {
    let args = function.args();
    let mut out = Vec::new();
    for arg in &args.posonlyargs {
        // Upstream 0.9.1's default mapping is built from `args.args` only, so
        // positional-only defaults are intentionally omitted.
        out.push(arg_with_default(source, arg, false));
    }
    for arg in &args.args {
        out.push(arg_with_default(source, arg, options.check_arg_defaults));
    }
    if let Some(arg) = &args.vararg {
        out.push(ActualArg {
            name: format!("*{}", arg.arg),
            ty: arg
                .annotation
                .as_deref()
                .map(|expr| canonical_annotation_text(source, expr))
                .unwrap_or_default(),
        });
    }
    for arg in &args.kwonlyargs {
        out.push(arg_with_default(source, arg, options.check_arg_defaults));
    }
    if let Some(arg) = &args.kwarg {
        out.push(ActualArg {
            name: format!("**{}", arg.arg),
            ty: arg
                .annotation
                .as_deref()
                .map(|expr| canonical_annotation_text(source, expr))
                .unwrap_or_default(),
        });
    }

    if matches!(parent, ParentDef::Class(_)) && !is_staticmethod(function) && !out.is_empty() {
        out.remove(0);
    }
    if options.ignore_underscore_args {
        out.retain(|arg| !arg.name.chars().all(|ch| ch == '_'));
    }
    if options.ignore_private_args {
        out.retain(|arg| !arg.name.starts_with('_') || arg.name.chars().all(|ch| ch == '_'));
    }
    out
}

fn arg_with_default(source: &str, arg: &ast::ArgWithDefault, include_default: bool) -> ActualArg {
    let mut ty = arg
        .def
        .annotation
        .as_deref()
        .map(|expr| canonical_annotation_text(source, expr))
        .unwrap_or_default();
    if include_default {
        if let Some(default) = &arg.default {
            append_default(&mut ty, &canonical_expression_text(default.as_ref()));
        }
    }
    ActualArg {
        name: arg.def.arg.to_string(),
        ty,
    }
}

fn add_stars_to_documented_args(documented: &mut [DocItem], actual: &[ActualArg]) {
    let star_names = actual
        .iter()
        .filter(|arg| arg.name.starts_with('*'))
        .map(|arg| {
            (
                arg.name.trim_start_matches('*').to_string(),
                arg.name.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    for arg in documented {
        if arg.name.starts_with('*') {
            continue;
        }
        if let Some(starred) = star_names.get(arg.name.trim_start_matches('*')) {
            arg.name = starred.clone();
        }
    }
}

fn doc_style_name(style: DocStyle) -> &'static str {
    match style {
        DocStyle::Google => "google",
        DocStyle::Numpy => "numpy",
        DocStyle::Sphinx => "sphinx",
    }
}

fn format_actual_arg(arg: &ActualArg) -> String {
    format!("{}: {}", arg.name, arg.ty)
}

fn format_doc_item(arg: &DocItem) -> String {
    format!("{}: {}", arg.name, arg.ty)
}

fn function_prefix(function: FunctionRef<'_>, parent: ParentDef<'_>) -> String {
    match parent {
        ParentDef::Class(class) => format!("Method `{}.{}`", class.name, function.name()),
        _ => format!("Function `{}`", function.name()),
    }
}

fn is_private_name(name: &str) -> bool {
    name.starts_with('_') && !(name.starts_with("__") && name.ends_with("__"))
}

fn bare_decorator_matches(expr: &Expr, expected: &str) -> bool {
    matches!(expr, Expr::Name(name) if name.id.as_str() == expected)
}

fn is_staticmethod(function: FunctionRef<'_>) -> bool {
    // Match pydoclint's detectMethodType(): only bare-name classmethod/
    // staticmethod decorators count, and the decorator closest to the
    // function definition wins when they are stacked.
    for decorator in function.decorators().iter().rev() {
        if bare_decorator_matches(decorator, "classmethod") {
            return false;
        }
        if bare_decorator_matches(decorator, "staticmethod") {
            return true;
        }
    }
    false
}

fn is_property(function: FunctionRef<'_>) -> bool {
    function
        .decorators()
        .first()
        .is_some_and(is_property_decorator)
}

fn is_property_decorator(expr: &Expr) -> bool {
    bare_decorator_matches(expr, "property")
}

fn is_abstract(function: FunctionRef<'_>) -> bool {
    function
        .decorators()
        .first()
        .is_some_and(|decorator| bare_decorator_matches(decorator, "abstractmethod"))
}

fn same_string_set_with_equal_len(left: &[&str], right: &[&str]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let left = left
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let right = right
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    left == right
}

fn canonical_type_text(text: &str) -> String {
    let compact = normalize_type_text(text);
    let mut out = String::with_capacity(compact.len() + 8);
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in compact.chars() {
        if let Some(active) = quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                out.push(ch);
            }
            ',' => {
                out.push(',');
                out.push(' ');
            }
            '|' => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push('|');
                out.push(' ');
            }
            _ => out.push(ch),
        }
    }
    out.trim().to_string()
}

fn canonical_annotation_text(_source: &str, expr: &Expr) -> String {
    replace_tuple_bracket(&expr.to_string()).trim().to_string()
}

fn canonical_expression_text(expr: &Expr) -> String {
    replace_tuple_bracket(&expr.to_string()).trim().to_string()
}

fn replace_tuple_bracket(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let prefix_len = if text[i..].starts_with("tuple[*") || text[i..].starts_with("Tuple[*") {
            Some(7usize)
        } else {
            None
        };
        let Some(prefix_len) = prefix_len else {
            let ch = text[i..].chars().next().expect("valid UTF-8 boundary");
            out.push(ch);
            i += ch.len_utf8();
            continue;
        };
        if let Some(relative_end) = text[i + prefix_len..].find(",]") {
            let end = i + prefix_len + relative_end;
            out.push_str(&text[i..end]);
            out.push(']');
            i = end + 2;
        } else {
            out.push_str(&text[i..]);
            break;
        }
    }
    out
}

fn return_annotation_text(source: &str, expr: &Expr) -> String {
    upstream_strip_quotes(&canonical_annotation_text(source, expr))
}

fn canonical_subscript_slice_text(source: &str, expr: &Expr) -> String {
    match expr {
        Expr::Tuple(tuple) => format!(
            "({})",
            tuple
                .elts
                .iter()
                .map(|item| canonical_annotation_text(source, item))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => canonical_annotation_text(source, expr),
    }
}

fn types_equal(left: &str, right: &str) -> bool {
    let left = upstream_strip_quotes(&normalize_type_text(left));
    let right = upstream_strip_quotes(&normalize_type_text(right));
    upstream_special_equal(&left, &right)
}

fn upstream_strip_quotes(text: &str) -> String {
    let mut text = text.to_string();
    if text.len() >= 4 && text.starts_with("``") && text.ends_with("``") {
        text = text[2..text.len() - 2].to_string();
    } else if text.len() >= 2 && text.starts_with('`') && text.ends_with('`') {
        text = text[1..text.len() - 1].to_string();
    }

    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < chars.len() {
        if i + 8 <= chars.len() && chars[i..i + 8].iter().collect::<String>() == "Literal[" {
            let start = i;
            let mut depth = 0i32;
            while i < chars.len() {
                if chars[i] == '[' {
                    depth += 1;
                }
                if chars[i] == ']' {
                    depth -= 1;
                    i += 1;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                i += 1;
            }
            out.extend(chars[start..i].iter());
            continue;
        }
        let start = i;
        while i < chars.len() {
            if i + 8 <= chars.len() && chars[i..i + 8].iter().collect::<String>() == "Literal[" {
                break;
            }
            // Upstream regex's `[^L]+` segments stop on a capital L.
            if chars[i] == 'L' {
                break;
            }
            i += 1;
        }
        if start == i {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let segment: String = chars[start..i].iter().collect();
        if segment.contains(",default=") || segment.contains(", default=") {
            out.push_str(&segment);
        } else {
            out.extend(segment.chars().filter(|ch| !matches!(ch, '\'' | '"')));
        }
    }
    out
}

fn upstream_special_equal(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let mut left = left
        .split_once('#')
        .map_or(left, |(head, _)| head)
        .trim_end()
        .to_string();
    let mut right = right
        .split_once('#')
        .map_or(right, |(head, _)| head)
        .trim_end()
        .to_string();
    // Upstream removes ordinary spaces only when at least one side is
    // multiline. On a single line, `Tuple[int,str]` and `Tuple[int, str]`
    // are intentionally different for return/yield comparisons.
    if left.contains('\n') || right.contains('\n') {
        left = left.replace([' ', '\n'], "");
        right = right.replace([' ', '\n'], "");
    }
    if left.chars().count() != right.chars().count() {
        return false;
    }
    left.chars()
        .zip(right.chars())
        .all(|(a, b)| a == b || (matches!(a, '\'' | '"') && matches!(b, '\'' | '"')))
}

fn return_annotation_is_none_or_noreturn(annotation: Option<&Expr>, source: &str) -> bool {
    match annotation {
        Some(Expr::Constant(node)) if matches!(&node.value, Constant::None) => true,
        Some(expr) => expression_text(source, expr) == "NoReturn",
        None => false,
    }
}

fn pydoclint_has_yield_statements(body: &[Stmt]) -> bool {
    for stmt in body {
        let found = match stmt {
            Stmt::FunctionDef(_) | Stmt::AsyncFunctionDef(_) | Stmt::ClassDef(_) => false,
            Stmt::Expr(node) => {
                matches!(node.value.as_ref(), Expr::Yield(_) | Expr::YieldFrom(_))
            }
            Stmt::For(node) => {
                pydoclint_has_yield_statements(&node.body)
                    || pydoclint_has_yield_statements(&node.orelse)
            }
            Stmt::AsyncFor(node) => {
                pydoclint_has_yield_statements(&node.body)
                    || pydoclint_has_yield_statements(&node.orelse)
            }
            Stmt::While(node) => {
                pydoclint_has_yield_statements(&node.body)
                    || pydoclint_has_yield_statements(&node.orelse)
            }
            Stmt::If(node) => {
                pydoclint_has_yield_statements(&node.body)
                    || pydoclint_has_yield_statements(&node.orelse)
            }
            Stmt::With(node) => pydoclint_has_yield_statements(&node.body),
            Stmt::AsyncWith(node) => pydoclint_has_yield_statements(&node.body),
            Stmt::Match(node) => node
                .cases
                .iter()
                .any(|case| pydoclint_has_yield_statements(&case.body)),
            Stmt::Try(node) => {
                pydoclint_has_yield_statements(&node.body)
                    || node.handlers.iter().any(|handler| {
                        let ast::ExceptHandler::ExceptHandler(handler) = handler;
                        pydoclint_has_yield_statements(&handler.body)
                    })
                    || pydoclint_has_yield_statements(&node.orelse)
                    || pydoclint_has_yield_statements(&node.finalbody)
            }
            Stmt::TryStar(node) => {
                pydoclint_has_yield_statements(&node.body)
                    || node.handlers.iter().any(|handler| {
                        let ast::ExceptHandler::ExceptHandler(handler) = handler;
                        pydoclint_has_yield_statements(&handler.body)
                    })
                    || pydoclint_has_yield_statements(&node.orelse)
                    || pydoclint_has_yield_statements(&node.finalbody)
            }
            _ => false,
        };
        if found {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeneratorKind {
    Generator,
    AsyncGenerator,
}

fn generator_annotation_kind(expr: &Expr) -> Option<GeneratorKind> {
    let name = match expr {
        Expr::Name(node) => Some(node.id.as_str()),
        Expr::Subscript(node) => match node.value.as_ref() {
            Expr::Name(name) => Some(name.id.as_str()),
            _ => None,
        },
        _ => None,
    }?;
    match name {
        "Generator" => Some(GeneratorKind::Generator),
        "AsyncGenerator" => Some(GeneratorKind::AsyncGenerator),
        _ => None,
    }
}

fn is_iterator_or_iterable_annotation(source: &str, expr: &Expr) -> bool {
    // pydoclint 0.9.1 performs a raw `ast.unparse(...).startswith(...)`
    // check. That deliberately recognizes odd spellings such as
    // `Iterator.foo`, `Iterator()`, and `Iterator123[int]`, while
    // `typing.Iterator[int]` does not match.
    let annotation = canonical_annotation_text(source, expr);
    ["Iterator", "Iterable", "AsyncIterator", "AsyncIterable"]
        .iter()
        .any(|prefix| annotation.starts_with(prefix))
}

fn subscript_args(expr: &Expr) -> Option<Vec<&Expr>> {
    let Expr::Subscript(node) = expr else {
        return None;
    };
    match node.slice.as_ref() {
        Expr::Tuple(tuple) => Some(tuple.elts.iter().collect()),
        other => Some(vec![other]),
    }
}

fn extract_yield_type(source: &str, expr: &Expr) -> Option<String> {
    if let Some(kind) = generator_annotation_kind(expr) {
        let Some(args) = subscript_args(expr) else {
            return Some(return_annotation_text(source, expr));
        };
        let max = match kind {
            GeneratorKind::Generator => 3,
            GeneratorKind::AsyncGenerator => 2,
        };
        if args.is_empty() || args.len() > max {
            return Some(return_annotation_text(source, expr));
        }
        return Some(upstream_strip_quotes(&canonical_annotation_text(
            source, args[0],
        )));
    }
    if is_iterator_or_iterable_annotation(source, expr) {
        let Expr::Subscript(node) = expr else {
            return Some(return_annotation_text(source, expr));
        };
        return Some(upstream_strip_quotes(&canonical_subscript_slice_text(
            source,
            &node.slice,
        )));
    }
    None
}

fn extract_generator_return_type(source: &str, expr: &Expr) -> Option<String> {
    let kind = generator_annotation_kind(expr)?;
    let Some(args) = subscript_args(expr) else {
        return Some(return_annotation_text(source, expr));
    };
    if kind == GeneratorKind::AsyncGenerator {
        return if (1..=2).contains(&args.len()) {
            Some("None".to_string())
        } else {
            Some(return_annotation_text(source, expr))
        };
    }
    if !(1..=3).contains(&args.len()) {
        return Some(return_annotation_text(source, expr));
    }
    if args.len() < 3 {
        Some("None".to_string())
    } else {
        Some(upstream_strip_quotes(&canonical_annotation_text(
            source, args[2],
        )))
    }
}

fn return_type_mismatch_message(
    style: DocStyle,
    annotation: &str,
    documented: &[String],
) -> Option<String> {
    if style != DocStyle::Numpy {
        let Some(documented_first) = documented.first() else {
            return (!annotation.is_empty()).then(|| {
                "Return annotation has 1 type(s); docstring return section has 0 type(s)."
                    .to_string()
            });
        };

        let documented_first = upstream_strip_quotes(documented_first);
        if annotation.is_empty() {
            // Google/Sphinx helpers only inspect the first parsed return item.
            return Some(
                "Return annotation has 0 type(s); docstring return section has 1 type(s)."
                    .to_string(),
            );
        }
        if upstream_special_equal(&documented_first, annotation) {
            return None;
        }
        return Some(format!(
            "Return annotation types: {}; docstring return section types: {}",
            python_list(&[annotation.to_string()]),
            python_list(&[documented_first])
        ));
    }

    let return_annotation_in_list = if annotation.is_empty() {
        Vec::new()
    } else {
        vec![annotation.to_string()]
    };
    let return_section_types = documented
        .iter()
        .map(|item| upstream_strip_quotes(item))
        .collect::<Vec<_>>();
    if return_annotation_in_list == return_section_types {
        return None;
    }

    let decomposed = decompose_tuple_annotation(annotation);
    if decomposed.len() != return_section_types.len() {
        return Some(format!(
            "Return annotation has {} type(s); docstring return section has {} type(s).",
            decomposed.len(),
            return_section_types.len()
        ));
    }
    if decomposed
        .iter()
        .zip(&return_section_types)
        .all(|(left, right)| upstream_special_equal(left, right))
    {
        None
    } else {
        Some(format!(
            "Return annotation types: {}; docstring return section types: {}",
            python_list(&decomposed),
            python_list(&return_section_types)
        ))
    }
}

fn decompose_tuple_annotation(annotation: &str) -> Vec<String> {
    if annotation.is_empty() {
        return Vec::new();
    }
    let normalized = annotation.to_string();
    let Some(body) = normalized
        .strip_prefix("Tuple[")
        .and_then(|body| body.strip_suffix(']'))
        .or_else(|| {
            normalized
                .strip_prefix("tuple[")
                .and_then(|body| body.strip_suffix(']'))
        })
    else {
        return vec![normalized];
    };
    if body.is_empty() {
        return Vec::new();
    }
    if body.trim_end().ends_with("...") {
        return vec![normalized];
    }
    split_top_level_commas(body)
        .into_iter()
        .map(|item| {
            let item = item.trim();
            ast::Expr::parse(item, "<annotation>")
                .map(|expr| canonical_expression_text(&expr))
                .unwrap_or_else(|_| canonical_type_text(item))
        })
        .collect()
}

fn yield_type_mismatch_message(
    original_annotation: Option<&str>,
    expected: Option<&str>,
    documented: &[String],
    recognized_generator_or_iterable: bool,
    require_none: bool,
) -> Option<String> {
    match documented.first() {
        Some(documented) => {
            if !recognized_generator_or_iterable || original_annotation.is_none() {
                return Some(
                    "Return annotation does not exist or is not Generator[...]/Iterator[...]/Iterable[...], but docstring \"yields\" section has 1 type(s).".to_string(),
                );
            }
            if expected != Some(documented.as_str()) {
                return Some(format!(
                    "The yield type (the 0th arg in Generator[...]/Iterator[...]): {}; docstring \"yields\" section types: {}",
                    expected.unwrap_or("None"),
                    documented
                ));
            }
            None
        }
        None => {
            if recognized_generator_or_iterable && expected == Some("None") && !require_none {
                None
            } else if original_annotation != Some("") {
                Some(
                    "Return annotation exists, but docstring \"yields\" section does not exist or has 0 type(s).".to_string(),
                )
            } else {
                None
            }
        }
    }
}

fn python_repr_string(value: &str) -> String {
    let escaped_backslashes = value.replace('\\', "\\\\");
    if value.contains('\'') && !value.contains('"') {
        format!("\"{escaped_backslashes}\"")
    } else {
        format!("'{}'", escaped_backslashes.replace('\'', "\\'"))
    }
}
fn python_list(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| python_repr_string(value))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn split_top_level_commas(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                let item = text[start..index].trim();
                if !item.is_empty() {
                    result.push(item.to_string());
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        result.push(tail.to_string());
    }
    result
}

fn raises_match(actual: &[String], documented: &[String]) -> bool {
    actual.len() == documented.len()
        && actual
            .iter()
            .zip(documented)
            .all(|(actual, documented)| actual.starts_with(documented))
}

fn collect_assignment_names(_source: &str, expr: &Expr, out: &mut Vec<ActualArg>) {
    match expr {
        Expr::Tuple(tuple) => {
            // Upstream expands an assignment tuple exactly one level. A nested
            // tuple remains a single synthetic attribute name, e.g. `(b, c)`.
            for item in &tuple.elts {
                out.push(ActualArg {
                    name: canonical_expression_text(item),
                    ty: String::new(),
                });
            }
        }
        _ => out.push(ActualArg {
            name: canonical_expression_text(expr),
            ty: String::new(),
        }),
    }
}

fn assignment_single_name(source: &str, stmt: &Stmt) -> Option<String> {
    match stmt {
        Stmt::AnnAssign(assign) => Some(canonical_expression_text(assign.target.as_ref())),
        Stmt::Assign(assign) => {
            let mut args = Vec::new();
            for target in &assign.targets {
                collect_assignment_names(source, target, &mut args);
            }
            (args.len() == 1).then(|| args.remove(0).name)
        }
        _ => None,
    }
}

fn classvar_inner_type(source: &str, annotation: &Expr) -> Option<String> {
    let Expr::Subscript(subscript) = annotation else {
        return None;
    };
    let Expr::Name(name) = subscript.value.as_ref() else {
        return None;
    };
    if name.id.as_str() != "ClassVar" {
        return None;
    }
    Some(canonical_annotation_text(source, &subscript.slice))
}

fn dataclass_field_to_actual(field: DataclassField, include_default: bool) -> ActualArg {
    let mut ty = field.ty;
    if include_default {
        if let Some(default) = field.default.as_deref() {
            append_default(&mut ty, default);
        }
    }
    ActualArg {
        name: field.name,
        ty,
    }
}

fn append_default(ty: &mut String, default: &str) {
    ty.push_str(", default=");
    ty.push_str(default);
}

fn find_pyproject(path: &Path) -> Option<PathBuf> {
    let mut directory = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };
    loop {
        let candidate = directory.join("pyproject.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !directory.pop() {
            return None;
        }
    }
}

pub fn validate_pydoclint_config_values_for_path(
    path: &Path,
    inferred_config_context: Option<&Path>,
    explicit_config: Option<&Path>,
    overrides: &PydoclintCliOverrides,
) -> Result<(), String> {
    let project_context = inferred_config_context.unwrap_or(path);
    let mut values = HashMap::<String, String>::new();

    if let Some(pyproject) = find_pyproject(project_context) {
        if let Ok(text) = fs::read_to_string(pyproject) {
            update_validated_config_values(&text, &mut values);
        }
    }
    if let Some(explicit_config) = explicit_config {
        if let Ok(text) = fs::read_to_string(explicit_config) {
            update_validated_config_values(&text, &mut values);
        }
    }

    if overrides.style.is_none() {
        if let Some(value) = values.get("style") {
            if DocStyle::parse(value).is_none() {
                return Err(format!(
                    "Invalid value for `--style`: `{value}`; expected `numpy`, `google`, or `sphinx`"
                ));
            }
        }
    }
    if overrides.native_mode_noqa_location.is_none() {
        if let Some(value) = values.get("native_mode_noqa_location") {
            if !matches!(
                value.to_ascii_lowercase().as_str(),
                "definition" | "docstring"
            ) {
                return Err(format!(
                    "Invalid value for `--native-mode-noqa-location`: `{value}`; expected `definition` or `docstring`"
                ));
            }
        }
    }

    macro_rules! validate_bool {
        ($key:literal, $override:expr) => {
            if $override.is_none() {
                if let Some(value) = values.get($key) {
                    if parse_click_bool(value).is_none() {
                        let option = $key.replace('_', "-");
                        return Err(format!(
                            "Invalid value for `--{option}`: `{value}`; expected a boolean value"
                        ));
                    }
                }
            }
        };
    }
    validate_bool!(
        "arg_type_hints_in_signature",
        overrides.arg_type_hints_in_signature
    );
    validate_bool!(
        "arg_type_hints_in_docstring",
        overrides.arg_type_hints_in_docstring
    );
    validate_bool!("check_arg_order", overrides.check_arg_order);
    validate_bool!(
        "skip_checking_short_docstrings",
        overrides.skip_checking_short_docstrings
    );
    validate_bool!("skip_checking_raises", overrides.skip_checking_raises);
    validate_bool!(
        "skip_checking_private_functions",
        overrides.skip_checking_private_functions
    );
    validate_bool!("allow_init_docstring", overrides.allow_init_docstring);
    validate_bool!("check_return_types", overrides.check_return_types);
    validate_bool!("check_yield_types", overrides.check_yield_types);
    validate_bool!("ignore_underscore_args", overrides.ignore_underscore_args);
    validate_bool!("ignore_private_args", overrides.ignore_private_args);
    validate_bool!("check_class_attributes", overrides.check_class_attributes);
    validate_bool!(
        "should_document_private_class_attributes",
        overrides.should_document_private_class_attributes
    );
    validate_bool!(
        "treat_property_methods_as_class_attributes",
        overrides.treat_property_methods_as_class_attributes
    );
    validate_bool!(
        "only_attrs_with_classvar_are_treated_as_class_attrs",
        overrides.only_attrs_with_classvar_are_treated_as_class_attrs
    );
    validate_bool!(
        "require_inline_class_var_docs",
        overrides.require_inline_class_var_docs
    );
    validate_bool!(
        "require_return_section_when_returning_nothing",
        overrides.require_return_section_when_returning_nothing
    );
    validate_bool!(
        "require_yield_section_when_yielding_nothing",
        overrides.require_yield_section_when_yielding_nothing
    );
    validate_bool!(
        "should_document_star_arguments",
        overrides.should_document_star_arguments
    );
    validate_bool!(
        "omit_stars_when_documenting_varargs",
        overrides.omit_stars_when_documenting_varargs
    );
    validate_bool!(
        "should_declare_assert_error_if_assert_statement_exists",
        overrides.should_declare_assert_error_if_assert_statement_exists
    );
    validate_bool!("check_style_mismatch", overrides.check_style_mismatch);
    validate_bool!("check_arg_defaults", overrides.check_arg_defaults);

    Ok(())
}

fn update_validated_config_values(text: &str, values: &mut HashMap<String, String>) {
    const VALIDATED_KEYS: &[&str] = &[
        "style",
        "arg_type_hints_in_signature",
        "arg_type_hints_in_docstring",
        "check_arg_order",
        "skip_checking_short_docstrings",
        "skip_checking_raises",
        "skip_checking_private_functions",
        "allow_init_docstring",
        "check_return_types",
        "check_yield_types",
        "ignore_underscore_args",
        "ignore_private_args",
        "check_class_attributes",
        "should_document_private_class_attributes",
        "treat_property_methods_as_class_attributes",
        "only_attrs_with_classvar_are_treated_as_class_attrs",
        "require_inline_class_var_docs",
        "require_return_section_when_returning_nothing",
        "require_yield_section_when_yielding_nothing",
        "should_document_star_arguments",
        "omit_stars_when_documenting_varargs",
        "should_declare_assert_error_if_assert_statement_exists",
        "check_style_mismatch",
        "check_arg_defaults",
        "native_mode_noqa_location",
    ];
    for section in ["tool.pydoclint", "tool.sklint.pydoclint"] {
        for key in VALIDATED_KEYS {
            if let Some(value) = toml_section_scalar(text, section, key) {
                values.insert((*key).to_string(), value);
            }
        }
    }
}

fn toml_section_scalar(text: &str, section: &str, wanted_key: &str) -> Option<String> {
    let expected = format!("[{section}]");
    let mut active = false;
    let mut result = None;
    for raw in text.lines() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            active = line == expected;
            continue;
        }
        if !active {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().replace('-', "_").to_ascii_lowercase();
        if key == wanted_key {
            result = Some(trim_toml_string(value.trim()).to_string());
        }
    }
    result
}

fn parse_click_bool(text: &str) -> Option<bool> {
    match trim_toml_string(text).to_ascii_lowercase().as_str() {
        "1" | "true" | "t" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "f" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn apply_native_toml_section(options: &mut PydoclintNativeOptions, text: &str, section: &str) {
    let expected = format!("[{section}]");
    let mut active = false;
    for raw in text.lines() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            active = line == expected;
            continue;
        }
        if !active {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().replace('-', "_").to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "quiet" => set_bool(&mut options.quiet, value),
            "exclude" => options.exclude = trim_toml_string(value).to_string(),
            "baseline" => {
                let value = trim_toml_string(value);
                options.baseline = (!value.is_empty()).then(|| PathBuf::from(value));
            }
            "generate_baseline" => set_bool(&mut options.generate_baseline, value),
            "auto_regenerate_baseline" => set_bool(&mut options.auto_regenerate_baseline, value),
            "show_filenames_in_every_violation_message" => {
                set_bool(
                    &mut options.show_filenames_in_every_violation_message,
                    value,
                );
                options.show_filenames_configured = true;
            }
            _ => {}
        }
    }
}

fn apply_toml_section(options: &mut PydoclintOptions, text: &str, section: &str) {
    let expected = format!("[{section}]");
    let mut active = false;
    for raw in text.lines() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            active = line == expected;
            continue;
        }
        if !active {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().replace('-', "_").to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "style" => {
                if let Some(style) = DocStyle::parse(trim_toml_string(value)) {
                    options.style = style;
                }
            }
            "arg_type_hints_in_signature" => {
                set_bool(&mut options.arg_type_hints_in_signature, value)
            }
            "arg_type_hints_in_docstring" => {
                set_bool(&mut options.arg_type_hints_in_docstring, value)
            }
            "check_arg_order" => set_bool(&mut options.check_arg_order, value),
            "skip_checking_short_docstrings" => {
                set_bool(&mut options.skip_checking_short_docstrings, value)
            }
            "skip_checking_raises" => set_bool(&mut options.skip_checking_raises, value),
            "skip_checking_private_functions" => {
                set_bool(&mut options.skip_checking_private_functions, value)
            }
            "allow_init_docstring" => set_bool(&mut options.allow_init_docstring, value),
            "check_return_types" => set_bool(&mut options.check_return_types, value),
            "check_yield_types" => set_bool(&mut options.check_yield_types, value),
            "ignore_underscore_args" => set_bool(&mut options.ignore_underscore_args, value),
            "ignore_private_args" => set_bool(&mut options.ignore_private_args, value),
            "check_class_attributes" => set_bool(&mut options.check_class_attributes, value),
            "should_document_private_class_attributes" => {
                set_bool(&mut options.should_document_private_class_attributes, value)
            }
            "treat_property_methods_as_class_attributes" => set_bool(
                &mut options.treat_property_methods_as_class_attributes,
                value,
            ),
            "only_attrs_with_classvar_are_treated_as_class_attrs" => set_bool(
                &mut options.only_attrs_with_classvar_are_treated_as_class_attrs,
                value,
            ),
            "require_inline_class_var_docs" => {
                set_bool(&mut options.require_inline_class_var_docs, value)
            }
            "require_return_section_when_returning_nothing" => set_bool(
                &mut options.require_return_section_when_returning_nothing,
                value,
            ),
            "require_yield_section_when_yielding_nothing" => set_bool(
                &mut options.require_yield_section_when_yielding_nothing,
                value,
            ),
            "should_document_star_arguments" => {
                set_bool(&mut options.should_document_star_arguments, value)
            }
            "omit_stars_when_documenting_varargs" => {
                set_bool(&mut options.omit_stars_when_documenting_varargs, value)
            }
            "should_declare_assert_error_if_assert_statement_exists" => set_bool(
                &mut options.should_declare_assert_error_if_assert_statement_exists,
                value,
            ),
            "check_style_mismatch" => set_bool(&mut options.check_style_mismatch, value),
            "check_arg_defaults" => set_bool(&mut options.check_arg_defaults, value),
            "native_mode_noqa_location" => {
                options.native_mode_noqa_location =
                    match trim_toml_string(value).to_ascii_lowercase().as_str() {
                        "definition" => NoqaLocation::Definition,
                        _ => NoqaLocation::Docstring,
                    };
            }
            _ => {}
        }
    }
}

fn set_bool(target: &mut bool, text: &str) {
    if let Some(value) = parse_click_bool(text) {
        *target = value;
    }
}

fn trim_toml_string(text: &str) -> &str {
    text.trim().trim_matches(|ch| matches!(ch, '\'' | '"'))
}

fn strip_toml_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '#' => return &line[..index],
            _ => {}
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EffectiveConfig, FileInlineConfig, PyProjectConfig, VscodeConfig};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn strict_config() -> EffectiveConfig {
        EffectiveConfig::resolve(
            &VscodeConfig::default(),
            &PyProjectConfig::default(),
            &FileInlineConfig {
                strict: Some(true),
                ..FileInlineConfig::default()
            },
        )
    }

    fn codes(source: &str) -> Vec<String> {
        // These unit fixtures are written in NumPy style. Product defaults
        // intentionally remain Google; make the test style explicit instead
        // of relying on a never-executed historical assumption.
        codes_with_options(source, "style = 'numpy'")
    }

    fn diagnostics_with_options(source: &str, pydoclint_toml: &str) -> Vec<Diagnostic> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sklint-pydoclint-{unique}"));
        fs::create_dir_all(&root).expect("create temp project");
        fs::write(
            root.join("pyproject.toml"),
            format!("[tool.pydoclint]\n{pydoclint_toml}\n"),
        )
        .expect("write pyproject");
        let path = root.join("case.py");
        fs::write(&path, source).expect("write source");
        let diagnostics = run_pydoclint_rules(&path, source, &strict_config());
        let _ = fs::remove_dir_all(root);
        diagnostics
    }

    fn codes_with_options(source: &str, pydoclint_toml: &str) -> Vec<String> {
        diagnostics_with_options(source, pydoclint_toml)
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect()
    }

    #[test]
    fn semantic_toml_bool_values_follow_click_aliases() {
        let mut options = PydoclintOptions::default();
        options.apply_toml_text(
            "[tool.pydoclint]\ncheck-arg-order = 'yes'\ncheck-return-types = 'off'\n",
        );
        assert!(options.check_arg_order);
        assert!(!options.check_return_types);
    }

    #[test]
    fn native_options_parse_from_pydoclint_toml() {
        let mut options = PydoclintNativeOptions::default();
        options.apply_toml_text(
            r#"
[tool.pydoclint]
quiet = true
exclude = '\.git|tests/data'
baseline = 'pydoclint-baseline.txt'
generate-baseline = true
auto-regenerate-baseline = false
show-filenames-in-every-violation-message = true
"#,
        );
        assert!(options.quiet);
        assert_eq!(options.exclude, r"\.git|tests/data");
        assert_eq!(
            options.baseline,
            Some(PathBuf::from("pydoclint-baseline.txt"))
        );
        assert!(options.generate_baseline);
        assert!(!options.auto_regenerate_baseline);
        assert!(options.show_filenames_in_every_violation_message);
    }

    #[test]
    fn sklint_pydoclint_native_section_overrides_upstream_section() {
        let mut options = PydoclintNativeOptions::default();
        options.apply_toml_text(
            r#"
[tool.pydoclint]
exclude = 'first'
quiet = false

[tool.sklint.pydoclint]
exclude = 'second'
quiet = true
"#,
        );
        assert_eq!(options.exclude, "second");
        assert!(options.quiet);
    }

    #[test]
    fn semantic_precedence_is_fallback_then_project_then_explicit_then_cli() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sklint-pydoclint-precedence-{unique}"));
        fs::create_dir_all(&root).expect("create temp project");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.pydoclint]\nstyle = 'numpy'\ncheck-arg-order = false\n",
        )
        .expect("write project config");
        let explicit = root.join("explicit.toml");
        fs::write(
            &explicit,
            "[tool.pydoclint]\nstyle = 'google'\ncheck-arg-order = false\n",
        )
        .expect("write explicit config");
        let file = root.join("case.py");
        fs::write(&file, "def f():\n    pass\n").expect("write source");
        let overrides = PydoclintCliOverrides {
            style: Some(DocStyle::Sphinx),
            check_arg_order: Some(true),
            ..PydoclintCliOverrides::default()
        };
        let options = PydoclintOptions::load_for_path_with_config(
            &file,
            DocStyle::Google,
            Some(&explicit),
            &overrides,
        );
        assert_eq!(options.style, DocStyle::Sphinx);
        assert!(options.check_arg_order);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn inferred_config_context_prevents_per_file_pydoclint_rediscovery() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sklint-pydoclint-common-{unique}"));
        let child = root.join("child");
        fs::create_dir_all(&child).expect("create nested project");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.pydoclint]\ncheck-arg-order = false\n",
        )
        .expect("write common config");
        fs::write(
            child.join("pyproject.toml"),
            "[tool.pydoclint]\ncheck-arg-order = true\n",
        )
        .expect("write child config");
        let file = child.join("case.py");
        fs::write(&file, "def f():\n    pass\n").expect("write source");

        let per_file = PydoclintOptions::load_for_path_with_config(
            &file,
            DocStyle::Google,
            None,
            &PydoclintCliOverrides::default(),
        );
        assert!(per_file.check_arg_order);

        let shared = PydoclintOptions::load_for_path_with_config_context(
            &file,
            DocStyle::Google,
            Some(&root),
            None,
            &PydoclintCliOverrides::default(),
        );
        assert!(!shared.check_arg_order);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn incompatible_sphinx_arg_defaults_are_rejected_after_all_layers() {
        let overrides = PydoclintCliOverrides {
            style: Some(DocStyle::Sphinx),
            check_arg_defaults: Some(true),
            ..PydoclintCliOverrides::default()
        };
        let options = PydoclintOptions::load_for_path_with_config(
            Path::new("does-not-exist.py"),
            DocStyle::Google,
            None,
            &overrides,
        );
        assert_eq!(options.style, DocStyle::Sphinx);
        assert!(options.check_arg_defaults);
        let error = options
            .validate()
            .expect_err("Sphinx defaults must be rejected");
        assert!(error.contains("not compatible"));
    }

    #[test]
    fn skd608_google_empty_parameter_description_is_reported() {
        let source = r#"
def f(value: int) -> None:
    """Summary.

    Args:
        value (int):
    """
"#;
        let codes = codes_with_options(
            source,
            "style = 'google'\nskip-checking-short-docstrings = false",
        );
        assert!(codes.contains(&"SKD608".to_string()));
    }

    #[test]
    fn skd608_google_multiline_parameter_description_is_allowed() {
        let source = r#"
def f(value: int) -> None:
    """Summary.

    Args:
        value (int):
            Useful value.
    """
"#;
        let codes = codes_with_options(
            source,
            "style = 'google'\nskip-checking-short-docstrings = false",
        );
        assert!(!codes.contains(&"SKD608".to_string()));
    }

    #[test]
    fn skd608_numpy_empty_parameter_description_is_reported() {
        let source = r#"
def f(value: int) -> None:
    """Summary.

    Parameters
    ----------
    value : int

    Returns
    -------
    None
    """
"#;
        let codes = codes_with_options(
            source,
            "style = 'numpy'\nskip-checking-short-docstrings = false",
        );
        assert!(codes.contains(&"SKD608".to_string()));
    }

    #[test]
    fn skd608_sphinx_attribute_directive_does_not_describe_previous_param() {
        let source = r#"
def f(value: int) -> None:
    """Summary.

    :param int value:
    .. attribute :: ghost
        :type: str

        Attribute description.
    """
"#;
        let codes = codes_with_options(
            source,
            "style = 'sphinx'\nskip-checking-short-docstrings = false",
        );
        assert!(codes.contains(&"SKD608".to_string()));
    }

    #[test]
    fn nested_return_does_not_satisfy_outer_function() {
        let source = r#"
def outer():
    """
    Summary.

    Returns
    -------
    int
        Value.
    """

    def nested() -> int:
        return 1
"#;
        assert!(codes(source).contains(&"SKD202".to_string()));
    }

    #[test]
    fn multiline_numpy_argument_type_matches_signature() {
        let source = r#"
def f(value: dict[str, list[int]]) -> None:
    """
    Summary.

    Parameters
    ----------
    value : dict[
        str,
        list[int]
    ]
        Value.
    """
"#;
        let codes = codes(source);
        assert!(!codes.contains(&"SKD105".to_string()));
    }

    #[test]
    fn raw_docstring_markdown_backslash_does_not_break_type_comparison() {
        let source = r#"
def f(value: dict[str, list[int]]) -> None:
    r"""
    Summary.

    Parameters
    ----------
    value : dict[str, \
        list[int]]
        Value.
    """
"#;
        let codes = codes(source);
        assert!(!codes.contains(&"SKD105".to_string()));
    }

    #[test]
    fn inherited_dataclass_fields_feed_doc601_family() {
        let source = r#"
from dataclasses import dataclass

@dataclass
class Parent:
    """
    Parent.

    Attributes
    ----------
    parent : int
        Parent field.
    """

    parent: int

@dataclass
class Child(Parent):
    """
    Child.

    Attributes
    ----------
    child : str
        Child field.
    """

    child: str
"#;
        let codes = codes(source);
        assert!(codes.contains(&"SKD601".to_string()));
        assert!(codes.contains(&"SKD603".to_string()));
    }

    #[test]
    fn assert_only_doc504_uses_assertion_error_and_has_safe_section_fix() {
        let source = r#"
def f(value: int) -> None:
    """Summary.

    Parameters
    ----------
    value : int
        Value.
    """
    assert value > 0
"#;
        let diagnostics = diagnostics_with_options(
            source,
            "style = 'numpy'
should-declare-assert-error-if-assert-statement-exists = true",
        );
        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "SKD503"));
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SKD504")
            .expect("SKD504");
        let fix = diagnostic.fix.as_ref().expect("SKD504 safe section fix");
        assert!(fix.safe);
        assert!(fix.replacement.contains("Raises"));
        assert!(fix.replacement.contains("AssertionError"));
    }

    #[test]
    fn assert_only_does_not_emit_doc503() {
        let source = r#"
def f(value: int) -> None:
    """
    Summary.

    Parameters
    ----------
    value : int
        Value.
    """
    assert value > 0
"#;
        let codes = codes(source);
        assert!(!codes.contains(&"SKD503".to_string()));
    }

    #[test]
    fn tuple_named_reraise_matches_documented_exceptions() {
        let source = r#"
def f() -> None:
    """
    Summary.

    Raises
    ------
    TypeError
        Bad type.
    ValueError
        Bad value.
    """
    try:
        work()
    except (ValueError, TypeError) as exc:
        raise exc
"#;
        let codes = codes(source);
        assert!(!codes.contains(&"SKD503".to_string()));
    }

    #[test]
    fn pydoclint_config_accepts_documented_classvar_option_spelling() {
        let mut options = PydoclintOptions::default();
        apply_toml_section(
            &mut options,
            "[tool.pydoclint]\nonly-attrs-with-ClassVar-are-treated-as-class-attrs = true\n",
            "tool.pydoclint",
        );
        assert!(options.only_attrs_with_classvar_are_treated_as_class_attrs);
    }

    #[test]
    fn untyped_star_args_do_not_trigger_partial_type_hint_diagnostics() {
        let source = r#"
def f(value: int, *args, **kwargs) -> None:
    """Summary.

    Parameters
    ----------
    value : int
        Value.
    *args
        Extra positional values.
    **kwargs
        Extra keyword values.
    """
"#;
        let codes = codes(source);
        assert!(!codes.contains(&"SKD107".to_string()));
        assert!(!codes.contains(&"SKD110".to_string()));
    }

    #[test]
    fn omitted_vararg_stars_mismatch_by_default_like_upstream() {
        let source = r#"
def f(value: int, *args, **kwargs) -> None:
    """Summary.

    Args:
        value (int): Value.
        args: Extra positional values.
        kwargs: Extra keyword values.
    """
"#;
        let codes = codes_with_options(source, "style = 'google'");
        assert!(codes.contains(&"SKD103".to_string()));
        assert!(codes.contains(&"SKD110".to_string()));
    }

    #[test]
    fn omit_vararg_stars_matches_only_the_correct_star_kind() {
        let matching = r#"
def f(first: int, *args: int, **kwargs: str) -> None:
    """Summary.

    Args:
        first (int): First value.
        args (int): Extra positional values.
        kwargs (str): Extra keyword values.
    """
"#;
        let matching_codes = codes_with_options(
            matching,
            "style = 'google'\nomit-stars-when-documenting-varargs = true",
        );
        assert!(!matching_codes.contains(&"SKD103".to_string()));

        let wrong_kind = r#"
def f(*args: int) -> None:
    """Summary.

    Args:
        **args (int): Wrong vararg kind.
    """
"#;
        let wrong_codes = codes_with_options(
            wrong_kind,
            "style = 'google'\nomit-stars-when-documenting-varargs = true",
        );
        assert!(wrong_codes.contains(&"SKD103".to_string()));
    }

    #[test]
    fn only_untyped_star_arg_still_triggers_no_type_hints_diagnostic() {
        let source = r#"
def f(*args) -> None:
    """Summary.

    Parameters
    ----------
    *args
        Extra values.
    """
"#;
        let codes = codes(source);
        assert!(codes.contains(&"SKD106".to_string()));
        assert!(codes.contains(&"SKD109".to_string()));
        assert!(!codes.contains(&"SKD107".to_string()));
        assert!(!codes.contains(&"SKD110".to_string()));
    }

    #[test]
    fn extra_doc_arg_on_zero_arg_function_still_emits_doc106_like_upstream() {
        let source = r#"
def f() -> None:
    """Summary.

    Parameters
    ----------
    ghost : int
        Extra.
    """
"#;
        let codes = codes(source);
        assert!(codes.contains(&"SKD102".to_string()));
        assert!(codes.contains(&"SKD103".to_string()));
        assert!(codes.contains(&"SKD106".to_string()));
    }

    #[test]
    fn definition_noqa_mode_points_pydoclint_diagnostics_at_definition_line() {
        let source = r#"
def f() -> int:
    """Summary.

    Returns
    -------
    str
        Value.
    """
    return 1
"#;
        let diagnostics = diagnostics_with_options(
            source,
            "style = 'numpy'\nnative-mode-noqa-location = \"definition\"",
        );
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SKD203")
            .expect("SKD203");
        assert_eq!(diagnostic.suppression_line, Some(2));
    }

    #[test]
    fn malformed_google_docstring_emits_only_doc001_for_function() {
        let source = r#"
def f(value: int) -> int:
    """Summary.

    Args:
        value

    Returns:
        int: Value.
    """
    return value
"#;
        let codes = codes_with_options(source, "style = \"google\"");
        let pydoclint_codes = codes
            .into_iter()
            .filter(|code| code.starts_with("SKD"))
            .collect::<Vec<_>>();
        assert_eq!(pydoclint_codes, vec!["SKD001"]);
    }

    #[test]
    fn style_mismatch_keeps_detected_argument_names() {
        let source = r#"
def f(value: int) -> None:
    """Summary.

    Args:
        value (int): Value.
    """
"#;
        let codes = codes_with_options(source, "style = \"numpy\"\ncheck-style-mismatch = true");
        assert!(codes.contains(&"SKD003".to_string()));
        assert!(!codes.contains(&"SKD101".to_string()));
        assert!(!codes.contains(&"SKD103".to_string()));
    }

    #[test]
    fn malformed_class_docstring_is_reported_when_init_docstrings_are_allowed() {
        let source = r#"
class Item:
    """Summary.

    Parameters
    ----------
        This has no parameter name.
    """

    def __init__(self, value: int) -> None:
        """Initialize.

        Parameters
        ----------
        value : int
            Value.
        """
        self.value = value
"#;
        let diagnostics = diagnostics_with_options(
            source,
            "style = \"numpy\"\nallow-init-docstring = true\ncheck-class-attributes = false",
        );
        let class_doc001 = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "SKD001" && diagnostic.line == 2)
            .count();
        assert_eq!(class_doc001, 1);
    }

    #[test]
    fn sphinx_attribute_directive_feeds_doc601_family() {
        let source = r#"
class Item:
    """Summary.

    .. attribute :: value
        :type: int

        Value.
    """

    value: int
"#;
        let codes = codes_with_options(source, "style = \"sphinx\"");
        assert!(!codes.contains(&"SKD601".to_string()));
        assert!(!codes.contains(&"SKD603".to_string()));
        assert!(!codes.contains(&"SKD605".to_string()));
    }

    #[test]
    fn sphinx_raises_type_in_description_matches_body_like_upstream() {
        let source = r#"
def f() -> None:
    """Summary.

    :raises: ValueError: Invalid value.
    """
    raise ValueError
"#;
        let codes = codes_with_options(source, "style = \"sphinx\"");
        assert!(!codes.contains(&"SKD503".to_string()));
    }

    #[test]
    fn duplicate_documented_raises_emit_doc503() {
        let source = r#"
def f() -> None:
    """Summary.

    Raises
    ------
    ValueError
        First.
    ValueError
        Duplicate.
    """
    raise ValueError
"#;
        assert!(codes(source).contains(&"SKD503".to_string()));
    }

    #[test]
    fn classvar_only_option_does_not_accept_qualified_classvar_like_upstream() {
        let source = r#"
import typing

class Item:
    """Summary.

    Attributes
    ----------
    value : int
        Value.
    """

    value: typing.ClassVar[int] = 1
"#;
        let codes = codes_with_options(
            source,
            "style = 'numpy'
only-attrs-with-ClassVar-are-treated-as-class-attrs = true",
        );
        assert!(codes.contains(&"SKD602".to_string()));
        assert!(codes.contains(&"SKD603".to_string()));
    }

    #[test]
    fn classvar_only_option_accepts_literal_classvar() {
        let source = r#"
from typing import ClassVar

class Item:
    """Summary.

    Attributes
    ----------
    value : int
        Value.
    """

    value: ClassVar[int] = 1
"#;
        let codes = codes_with_options(
            source,
            "only-attrs-with-ClassVar-are-treated-as-class-attrs = true",
        );
        assert!(!codes.contains(&"SKD601".to_string()));
        assert!(!codes.contains(&"SKD602".to_string()));
        assert!(!codes.contains(&"SKD603".to_string()));
        assert!(!codes.contains(&"SKD605".to_string()));
    }

    #[test]
    fn qualified_assignment_target_participates_in_class_attribute_checks() {
        let source = r#"
class Item:
    """Summary.

    Attributes
    ----------
    state.value
        Value.
    """

    state.value = 1
"#;
        let codes = codes(source);
        assert!(!codes.contains(&"SKD601".to_string()));
        assert!(!codes.contains(&"SKD603".to_string()));
    }

    #[test]
    fn argument_defaults_normalize_quotes_and_spacing_like_upstream() {
        let source = r#"
def f(value: str = "hello", count: int = 1) -> None:
    """Summary.

    Args:
        value (str, default='hello'): Value.
        count (int, default      =   1): Count.
    """
"#;
        let codes = codes_with_options(source, "style = 'google'\ncheck-arg-defaults = true");
        assert!(!codes.contains(&"SKD105".to_string()));
    }

    #[test]
    fn argument_defaults_reject_unquoted_string_and_noncanonical_syntax() {
        let source = r#"
def f(value: str = "hello", other: str = "world") -> None:
    """Summary.

    Args:
        value (str, default=hello): Unquoted string.
        other (str = "world"): Noncanonical default spelling.
    """
"#;
        let codes = codes_with_options(source, "style = 'google'\ncheck-arg-defaults = true");
        assert!(codes.contains(&"SKD105".to_string()));
    }

    #[test]
    fn class_attribute_defaults_participate_in_doc605() {
        let source = r#"
class Item:
    """Summary.

    Attributes
    ----------
    value : int, default=2
        Value.
    """

    value: int = 1
"#;
        let codes = codes_with_options(
            source,
            "style = 'numpy'
check-arg-defaults = true",
        );
        assert!(codes.contains(&"SKD605".to_string()));
    }

    #[test]
    fn matching_class_attribute_defaults_do_not_emit_doc605() {
        let source = r#"
class Item:
    """Summary.

    Attributes
    ----------
    value : int, default=1
        Value.
    """

    value: int = 1
"#;
        let codes = codes_with_options(source, "check-arg-defaults = true");
        assert!(!codes.contains(&"SKD605".to_string()));
    }

    #[test]
    fn untyped_function_default_uses_upstream_leading_comma_form() {
        let source = r#"
def f(value=1) -> None:
    """Summary.

    Parameters
    ----------
    value : , default=1
        Value.
    """
"#;
        let codes = codes_with_options(
            source,
            "check-arg-defaults = true\narg-type-hints-in-signature = false",
        );
        assert!(!codes.contains(&"SKD105".to_string()));
    }

    #[test]
    fn missing_returns_section_can_emit_doc201_and_doc203() {
        let source = r#"
def f(value: int) -> int:
    """Summary.

    Parameters
    ----------
    value : int
        Value.
    """
    return value
"#;
        let codes = codes(source);
        assert!(codes.contains(&"SKD201".to_string()));
        assert!(codes.contains(&"SKD203".to_string()));
    }

    #[test]
    fn pep696_two_arg_generator_does_not_require_returns_section() {
        let source = r#"
from typing import Generator

def f(value: int) -> Generator[int, str]:
    """Summary.

    Parameters
    ----------
    value : int
        Value.

    Yields
    ------
    int
        Value.
    """
    yield value
    return None
"#;
        let codes = codes(source);
        assert!(!codes.contains(&"SKD201".to_string()));
        assert!(!codes.contains(&"SKD203".to_string()));
        assert!(!codes.contains(&"SKD404".to_string()));
        assert!(!codes.contains(&"SKD405".to_string()));
    }

    #[test]
    fn iterator_value_return_without_returns_section_emits_only_doc201_from_mixed_family() {
        let source = r#"
from typing import Iterator

def f(value: int) -> Iterator[int]:
    """Summary.

    Parameters
    ----------
    value : int
        Value.

    Yields
    ------
    int
        Value.
    """
    yield value
    return "done"
"#;
        let codes = codes(source);
        assert!(codes.contains(&"SKD201".to_string()));
        assert!(!codes.contains(&"SKD203".to_string()));
        assert!(!codes.contains(&"SKD404".to_string()));
        assert!(!codes.contains(&"SKD405".to_string()));
    }

    #[test]
    fn bare_iterator_with_returns_and_yields_emits_doc404_and_doc405() {
        let source = r#"
from typing import Iterator

def f(value: int) -> Iterator:
    """Summary.

    Parameters
    ----------
    value : int
        Value.

    Returns
    -------
    str
        Done.

    Yields
    ------
    int
        Value.
    """
    yield value
    return "done"
"#;
        let codes = codes(source);
        assert!(codes.contains(&"SKD404".to_string()));
        assert!(codes.contains(&"SKD405".to_string()));
        assert!(!codes.contains(&"SKD201".to_string()));
        assert!(!codes.contains(&"SKD203".to_string()));
    }

    #[test]
    fn yield_without_return_annotation_emits_doc402_and_doc404() {
        let source = r#"
def f():
    """Summary."""
    yield 1
"#;
        let codes = codes_with_options(source, "skip-checking-short-docstrings = false");
        assert!(codes.contains(&"SKD402".to_string()));
        assert!(codes.contains(&"SKD404".to_string()));
    }

    #[test]
    fn bare_generator_without_yields_section_still_emits_doc404() {
        let source = r#"
from typing import Generator

def f() -> Generator:
    """Summary.

    Notes
    -----
    Generator with a deliberately bare annotation.
    """
    yield 1
"#;
        let codes = codes_with_options(
            source,
            "style = 'numpy'\nskip-checking-short-docstrings = false",
        );
        assert!(codes.contains(&"SKD402".to_string()));
        assert!(codes.contains(&"SKD404".to_string()));
    }
    #[test]
    fn skd002_uses_upstream_line_zero_for_syntax_errors() {
        let diagnostics = run_pydoclint_rules(
            Path::new("example.py"),
            "def broken(:\n    pass\n",
            &strict_config(),
        );
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SKD002")
            .expect("syntax error diagnostic");
        assert_eq!(diagnostic.line, 0);
        assert_eq!(diagnostic.end_line, 0);
    }

    #[test]
    fn invisible_characters_are_replaced_like_upstream_before_retry() {
        assert_eq!(replace_invisible_chars("a\u{feff}b"), "a b");
        assert_eq!(
            replace_invisible_chars(
                "a\u{200b}\u{200c}\u{200d}\u{2060}\u{180e}\u{061c}\u{200e}\u{200f}\u{202a}\u{202b}\u{202c}\u{202d}\u{202e}\u{2061}\u{2062}\u{2063}\u{2064}\u{034f}b"
            ),
            "ab"
        );
    }

    #[test]
    fn skd002_retries_after_invalid_non_printable_character() {
        let diagnostics = run_pydoclint_rules(
            Path::new("example.py"),
            "value\u{200b} = 1\n",
            &strict_config(),
        );
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD002"));
    }

    #[test]
    fn skd002_reports_second_parse_error_on_line_zero_after_invisible_retry() {
        let diagnostics = run_pydoclint_rules(
            Path::new("example.py"),
            "def broken(\u{200b}:\n    pass\n",
            &strict_config(),
        );
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SKD002")
            .expect("second syntax error diagnostic");
        assert_eq!(diagnostic.line, 0);
        assert_eq!(diagnostic.end_line, 0);
    }

    #[test]
    fn invisible_retry_disables_structural_fixes_with_shifted_offsets() {
        let source = "def\u{200b} f(value: int) -> None:\n    \"\"\"Summary.\n\n    Notes:\n        Existing section.\n    \"\"\"\n";
        let diagnostics = diagnostics_with_options(
            source,
            "style = 'numpy'\nskip-checking-short-docstrings = false",
        );
        let missing_args = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SKD101")
            .expect("missing argument section diagnostic");
        assert!(
            missing_args.fix.is_none(),
            "fix ranges must not be generated from the sanitized AST"
        );
    }
    fn decorator_flags(source: &str) -> (bool, bool, bool) {
        let ast = PythonAst::parse(source, "example.py").expect("valid Python");
        let function = match ast.suite.first() {
            Some(Stmt::FunctionDef(function)) => FunctionRef::Sync(function),
            Some(Stmt::AsyncFunctionDef(function)) => FunctionRef::Async(function),
            _ => panic!("expected function"),
        };
        (
            is_property(function),
            is_abstract(function),
            is_staticmethod(function),
        )
    }

    #[test]
    fn property_and_abstract_only_use_outermost_bare_decorator() {
        assert_eq!(
            decorator_flags("@property\ndef f(): ...\n"),
            (true, false, false)
        );
        assert_eq!(
            decorator_flags("@custom.property\ndef f(): ...\n"),
            (false, false, false)
        );
        assert_eq!(
            decorator_flags("@wrapper\n@property\ndef f(): ...\n"),
            (false, false, false)
        );
        assert_eq!(
            decorator_flags("@property\n@wrapper\ndef f(): ...\n"),
            (true, false, false)
        );
        assert_eq!(
            decorator_flags("@abstractmethod\ndef f(): ...\n"),
            (false, true, false)
        );
        assert_eq!(
            decorator_flags("@abc.abstractmethod\ndef f(): ...\n"),
            (false, false, false)
        );
        assert_eq!(
            decorator_flags("@wrapper\n@abstractmethod\ndef f(): ...\n"),
            (false, false, false)
        );
    }

    #[test]
    fn method_type_only_uses_bare_class_and_static_method_names() {
        assert_eq!(
            decorator_flags("@staticmethod\ndef f(): ...\n"),
            (false, false, true)
        );
        assert_eq!(
            decorator_flags("@custom.staticmethod\ndef f(): ...\n"),
            (false, false, false)
        );
        assert_eq!(
            decorator_flags("@staticmethod\n@classmethod\ndef f(): ...\n"),
            (false, false, false)
        );
        assert_eq!(
            decorator_flags("@classmethod\n@staticmethod\ndef f(): ...\n"),
            (false, false, true)
        );
    }
    #[test]
    fn generator_call_annotation_is_not_recognized_like_upstream() {
        let source = r#"
def f() -> Generator():
    """Summary.

    Yields:
        int: Value.
    """
    yield 1
"#;
        let diagnostics = run_pydoclint_rules(Path::new("example.py"), source, &strict_config());
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD403"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD404"));
    }

    #[test]
    fn iterator_prefix_annotation_is_recognized_like_upstream() {
        let source = r#"
def f() -> Iterator123[int]:
    """Summary.

    Yields:
        int: Value.
    """
    yield 1
"#;
        let diagnostics = run_pydoclint_rules(Path::new("example.py"), source, &strict_config());
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD403"));
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD404"));
    }

    #[test]
    fn iterator_multi_arg_yield_type_uses_raw_tuple_slice() {
        let source = r#"
def f() -> Iterator[int, str]:
    """Summary.

    Yields:
        int: Value.
    """
    yield 1
"#;
        let diagnostics = run_pydoclint_rules(Path::new("example.py"), source, &strict_config());
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SKD404")
            .expect("SKD404");
        assert!(diagnostic.message.contains("(int, str)"));
    }

    #[test]
    fn positional_only_defaults_are_omitted_from_upstream_default_mapping() {
        let source = r#"
def f(value: int = 1, /):
    """Summary.

    Parameters
    ----------
    value : int, default=1
        Value.
    """
"#;
        let diagnostics =
            diagnostics_with_options(source, "style = 'numpy'\ncheck-arg-defaults = true");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD105"));
    }

    #[test]
    fn doc203_message_uses_upstream_dynamic_postfix() {
        let source = r#"
def f() -> int:
    """Summary.

    Returns:
        str: Value.
    """
    return 1
"#;
        let diagnostics = run_pydoclint_rules(Path::new("example.py"), source, &strict_config());
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SKD203")
            .expect("SKD203");
        assert!(diagnostic
            .message
            .contains("Return annotation types: ['int']; docstring return section types: ['str']"));
    }

    #[test]
    fn zero_argument_function_does_not_emit_doc106_like_upstream() {
        let source = r#"
def f() -> None:
    """Summary.

    Notes
    -----
    Details.
    """
"#;
        let diagnostics = diagnostics_with_options(
            source,
            "style = \"numpy\"\nskip-checking-short-docstrings = false",
        );
        assert!(!diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.code.as_str(),
                "SKD106" | "SKD107" | "SKD109" | "SKD110"
            )
        }));
    }

    #[test]
    fn numpy_missing_return_annotation_has_zero_types_like_upstream() {
        let source = r#"
def f():
    """Summary.

    Notes
    -----
    Details.
    """
    return 1
"#;
        let diagnostics = diagnostics_with_options(
            source,
            "style = \"numpy\"\nskip-checking-short-docstrings = false",
        );
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD201"));
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD203"));
    }

    #[test]
    fn quoted_none_return_annotation_suppresses_doc203_but_not_doc201() {
        for annotation in ["'None'", "'NoReturn'"] {
            let source = format!(
                "def f() -> {annotation}:\n    \"\"\"Summary.\n\n    Notes\n    -----\n    Details.\n    \"\"\"\n"
            );
            let diagnostics = diagnostics_with_options(
                &source,
                "style = \"numpy\"\nskip-checking-short-docstrings = false",
            );
            assert!(diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "SKD201"));
            assert!(!diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "SKD203"));
        }
    }

    #[test]
    fn iterator_attribute_prefix_is_recognized_like_upstream() {
        let source = r#"
def f() -> Iterator.foo:
    """Summary.

    Yields
    ------
    Iterator.foo
        Value.
    """
    yield 1
"#;
        let diagnostics = diagnostics_with_options(source, "style = \"numpy\"");
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD403"));
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD404"));
    }

    #[test]
    fn forward_reference_yield_type_is_unquoted_like_upstream() {
        let source = r#"
def f() -> Generator["MyClass", None, None]:
    """Summary.

    Yields
    ------
    MyClass
        Value.
    """
    yield object()
"#;
        let diagnostics = diagnostics_with_options(source, "style = \"numpy\"");
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD404"));
    }

    #[test]
    fn assignment_form_yield_is_not_a_pydoclint_yield_statement() {
        let source = r#"
def f() -> Generator[int, None, None]:
    """Summary.

    Yields
    ------
    int
        Value.
    """
    value = yield 1
"#;
        let diagnostics = diagnostics_with_options(source, "style = \"numpy\"");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD403"));
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD404"));
    }

    #[test]
    fn statement_form_yield_still_counts_for_pydoclint() {
        let source = r#"
def f() -> Generator[int, None, None]:
    """Summary.

    Yields
    ------
    int
        Value.
    """
    yield 1
"#;
        let diagnostics = diagnostics_with_options(source, "style = \"numpy\"");
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD403"));
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD404"));
    }

    #[test]
    fn doc105_default_note_keeps_upstream_space_before_dot() {
        let source = r#"
def f(value: int = 1) -> None:
    """Summary.

    Parameters
    ----------
    value : int
        Value.
    """
"#;
        let diagnostics =
            diagnostics_with_options(source, "style = 'numpy'\ncheck-arg-defaults = true");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SKD105")
            .expect("SKD105");
        assert!(diagnostic
            .message
            .contains("value . (Note: docstring arg defaults should look like: `, default=XXX`)"));
    }

    #[test]
    fn constructor_discards_normal_returns_but_keeps_yields_check_like_upstream() {
        let source = r#"
class C:
    """Summary.

    Returns:
        None

    Yields:
        int
    """

    def __init__(self) -> None:
        pass
"#;
        let diagnostics = diagnostics_with_options(
            source,
            "style = \"google\"\nskip-checking-short-docstrings = false",
        );
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD203"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD403"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD302"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD306"));
    }

    #[test]
    fn constructor_structural_diagnostics_follow_upstream_order() {
        let source = r#"
class C:
    """Summary.

    Parameters
    ----------
    value : int
        Value.

    Returns
    -------
    int
        Value.

    Yields
    ------
    int
        Value.

    Raises
    ------
    ValueError
        Error.
    """

    def __init__(self, value: int) -> None:
        """Summary.

        Parameters
        ----------
        value : int
            Value.

        Returns
        -------
        int
            Value.

        Yields
        ------
        int
            Value.
        """
        self.value = value
"#;
        let diagnostics = diagnostics_with_options(
            source,
            "style = \"numpy\"\nallow-init-docstring = true\ncheck-class-attributes = false",
        );
        let codes = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>();
        let first = |code: &str| {
            codes
                .iter()
                .position(|candidate| *candidate == code)
                .expect(code)
        };
        assert!(first("SKD302") < first("SKD303"));
        assert!(first("SKD303") < first("SKD304"));
        assert!(first("SKD304") < first("SKD306"));
        assert!(first("SKD306") < first("SKD307"));
        assert!(first("SKD307") < first("SKD305"));
    }

    #[test]
    fn class_assignment_tuple_is_expanded_only_one_level() {
        let source = "a, (b, c) = 1, (2, 3)";
        let ast = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::Assign(assign) = ast.suite.first().expect("assignment") else {
            panic!("expected assignment");
        };
        let mut actual = Vec::new();
        for target in &assign.targets {
            collect_assignment_names(source, target, &mut actual);
        }
        let names = actual.into_iter().map(|arg| arg.name).collect::<Vec<_>>();
        assert_eq!(names, vec!["a", "(b, c)"]);
    }

    #[test]
    fn single_line_return_spacing_uses_upstream_special_equal() {
        let source = r#"
def f() -> Tuple[int, str]:
    """Summary.

    Returns
    -------
    Tuple[int,str]
        Value.
    """
    return 1, "x"
"#;
        let diagnostics = diagnostics_with_options(source, "style = \"numpy\"");
        // NumPy's compound form first compares the full annotation string
        // exactly before attempting tuple decomposition. With one documented
        // compound item, decomposition changes the count and DOC203 remains.
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD203"));
    }

    #[test]
    fn defaults_use_ast_unparse_like_upstream() {
        let source = r#"
def f(value: int = 0x10, mapping: dict = {"a":1}) -> None:
    """Summary.

    Parameters
    ----------
    value : int, default=16
        Value.
    mapping : dict, default={'a': 1}
        Mapping.
    """
"#;
        let diagnostics =
            diagnostics_with_options(source, "style = \"numpy\"\ncheck-arg-defaults = true");
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD105"));
    }

    #[test]
    fn class_attribute_defaults_use_ast_unparse_like_upstream() {
        let source = r#"
class C:
    """Summary.

    Attributes
    ----------
    mapping : dict, default={'a': 1}
        Mapping.
    """

    mapping: dict = {"a":1}
"#;
        let diagnostics =
            diagnostics_with_options(source, "style = \"numpy\"\ncheck-arg-defaults = true");
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD605"));
    }

    #[test]
    fn actual_annotations_use_ast_unparse_like_upstream() {
        let source = r#"
def f(value: (int)) -> (int):
    """Summary.

    Parameters
    ----------
    value : int
        Value.

    Returns
    -------
    int
        Value.
    """
    return value
"#;
        let diagnostics = diagnostics_with_options(source, "style = \"numpy\"");
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic.code.as_str(), "SKD105" | "SKD203")));
    }

    #[test]
    fn forward_tuple_return_elements_are_reparsed_and_unparsed_like_upstream() {
        let source = r#"
def f() -> "Tuple[(int), str]":
    """Summary.

    Returns
    -------
    int
        Number.
    str
        Text.
    """
    return 1, "x"
"#;
        let diagnostics = diagnostics_with_options(source, "style = \"numpy\"");
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "SKD203"));
    }

    #[test]
    fn pep646_tuple_star_comma_is_removed_like_upstream_py311_unparse() {
        let expr = ast::Expr::parse("tuple[*Shape]", "<annotation>").expect("valid annotation");
        assert_eq!(canonical_expression_text(&expr), "tuple[*Shape]");
    }

    #[test]
    fn doc603_inline_message_matches_upstream_wording_and_spacing() {
        let source = r#"
class C:
    """Summary."""

    value: int
"#;
        let diagnostics = diagnostics_with_options(
            source,
            "style = \"numpy\"\nrequire-inline-class-var-docs = true\nskip-checking-short-docstrings = false",
        );
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SKD603")
            .expect("SKD603");
        assert!(diagnostic.message.contains(
            "Attributes in the class definition but not documented inline: [value: int]. (Please read https://jsh9.github.io/pydoclint/checking_class_attributes.html on how to correctly document class attributes.)"
        ));
    }

    #[test]
    fn doc605_message_keeps_upstream_double_space_before_class_help() {
        let source = r#"
class C:
    """Summary.

    Attributes
    ----------
    value : str
        Value.
    """

    value: int
"#;
        let diagnostics = diagnostics_with_options(source, "style = \"numpy\"");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "SKD605")
            .expect("SKD605");
        assert!(diagnostic.message.contains(
            "do not match: value  (Please read https://jsh9.github.io/pydoclint/checking_class_attributes.html"
        ));
    }

    #[test]
    fn google_lossless_reorder_preserves_complete_argument_blocks() {
        let literal = r#""""Summary.

Args:
    second (str): Second line one.
        Second line two.
    first (int): First description.

Returns:
    str: Done.
""""#;
        let reordered =
            reorder_docstring_literal_items(literal, DocStyle::Google, false, &["first", "second"])
                .expect("safe reorder");
        assert!(reordered.find("first (int)").unwrap() < reordered.find("second (str)").unwrap());
        assert!(reordered.contains("Second line one.\n        Second line two."));
        assert!(reordered.contains("Returns:\n    str: Done."));
    }

    #[test]
    fn numpy_lossless_reorder_preserves_complete_attribute_blocks() {
        let literal = r#""""Summary.

Attributes
----------
second : str
    Second line one.
    Second line two.
first : int
    First description.

Notes
-----
Keep this section.
""""#;
        let reordered =
            reorder_docstring_literal_items(literal, DocStyle::Numpy, true, &["first", "second"])
                .expect("safe reorder");
        assert!(reordered.find("first : int").unwrap() < reordered.find("second : str").unwrap());
        assert!(reordered.contains("Second line one.\n    Second line two."));
        assert!(reordered.contains("Notes\n-----\nKeep this section."));
    }

    #[test]
    fn sphinx_reorder_is_intentionally_not_offered() {
        assert!(reorder_docstring_literal_items(
            "\"\"\"Summary.\n:param second: Second.\n:param first: First.\n\"\"\"",
            DocStyle::Sphinx,
            false,
            &["first", "second"],
        )
        .is_none());
    }

    #[test]
    fn reorder_refuses_duplicate_or_ambiguous_names() {
        let literal = "\"\"\"Summary.\n\nArgs:\n    value: One.\n    value: Two.\n\"\"\"";
        assert!(reorder_docstring_literal_items(
            literal,
            DocStyle::Google,
            false,
            &["value", "value"],
        )
        .is_none());
    }
}
