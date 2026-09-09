use crate::pydoclint_doc::DocStyle;
use crate::rules::{code_matches_selector, RuleLevel, ALL_RULES};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PydoclintCliOverrides {
    pub style: Option<DocStyle>,
    pub arg_type_hints_in_signature: Option<bool>,
    pub arg_type_hints_in_docstring: Option<bool>,
    pub check_arg_order: Option<bool>,
    pub skip_checking_short_docstrings: Option<bool>,
    pub skip_checking_raises: Option<bool>,
    pub skip_checking_private_functions: Option<bool>,
    pub allow_init_docstring: Option<bool>,
    pub check_return_types: Option<bool>,
    pub check_yield_types: Option<bool>,
    pub ignore_underscore_args: Option<bool>,
    pub ignore_private_args: Option<bool>,
    pub check_class_attributes: Option<bool>,
    pub should_document_private_class_attributes: Option<bool>,
    pub treat_property_methods_as_class_attributes: Option<bool>,
    pub only_attrs_with_classvar_are_treated_as_class_attrs: Option<bool>,
    pub require_inline_class_var_docs: Option<bool>,
    pub require_return_section_when_returning_nothing: Option<bool>,
    pub require_yield_section_when_yielding_nothing: Option<bool>,
    pub should_document_star_arguments: Option<bool>,
    pub omit_stars_when_documenting_varargs: Option<bool>,
    pub should_declare_assert_error_if_assert_statement_exists: Option<bool>,
    pub allow_documented_propagated_exceptions: Option<bool>,
    pub check_style_mismatch: Option<bool>,
    pub check_arg_defaults: Option<bool>,
    pub native_mode_noqa_location: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VscodeConfig {
    pub strict: Option<bool>,
    pub select: Vec<String>,
    pub ignore: Vec<String>,
    pub formatter_docstring_style: Option<DocStyle>,
    pub pydoclint_config_path: Option<PathBuf>,
    /// CLI-native inferred config context. When set, pydoclint semantics use
    /// one shared project config discovered from this common input context
    /// instead of rediscovering a different pydoclint config per file.
    pub pydoclint_inferred_config_context: Option<PathBuf>,
    pub pydoclint_overrides: PydoclintCliOverrides,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PyProjectConfig {
    pub found_path: Option<PathBuf>,
    pub has_sklint_section: bool,
    pub strict: Option<bool>,
    pub select: Vec<String>,
    pub ignore: Vec<String>,
    pub formatter_docstring_style: Option<DocStyle>,
    pub pydoclint_style: Option<DocStyle>,
    /// Additional assertion/oracle helper names or `*` patterns used by
    /// SK901. The patterns are matched against both the full qualified call
    /// name and its final component, so `verify_*` also matches
    /// `checks.verify_equal(...)`.
    pub assertion_helpers: Vec<String>,
    /// Additional function/method names or `*` patterns that define an
    /// explicit exception boundary for SK506.
    pub exception_boundary_functions: Vec<String>,
    /// Configuration errors discovered while parsing `[tool.sklint]`.
    /// CLI entry points fail-fast on these instead of silently linting with
    /// a partially applied configuration.
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileInlineConfig {
    pub strict: Option<bool>,
    pub select: Vec<String>,
    pub ignore: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveConfig {
    pub strict: bool,
    pub select: Vec<String>,
    pub ignore: Vec<String>,
    pub active_codes: Vec<String>,
    pub pyproject_path: Option<PathBuf>,
    pub formatter_docstring_style: DocStyle,
    pub pydoclint_config_path: Option<PathBuf>,
    pub pydoclint_inferred_config_context: Option<PathBuf>,
    pub pydoclint_overrides: PydoclintCliOverrides,
    pub assertion_helpers: Vec<String>,
    pub exception_boundary_functions: Vec<String>,
}

impl EffectiveConfig {
    pub fn resolve(
        vscode: &VscodeConfig,
        pyproject: &PyProjectConfig,
        inline: &FileInlineConfig,
    ) -> Self {
        let has_product_pyproject = pyproject.has_sklint_section;
        let base_strict = if has_product_pyproject {
            pyproject.strict.unwrap_or(false)
        } else {
            vscode.strict.unwrap_or(false)
        };
        let strict = inline.strict.unwrap_or(base_strict);

        let mut formatter_docstring_style = if has_product_pyproject {
            pyproject
                .formatter_docstring_style
                .unwrap_or(DocStyle::Google)
        } else {
            vscode.formatter_docstring_style.unwrap_or(DocStyle::Google)
        };
        if let Some(context) = vscode.pydoclint_inferred_config_context.as_deref() {
            if let Some(style) = load_pyproject_for_file(context).pydoclint_style {
                formatter_docstring_style = style;
            }
        } else if let Some(style) = pyproject.pydoclint_style {
            formatter_docstring_style = style;
        }
        if let Some(path) = vscode.pydoclint_config_path.as_deref() {
            if let Ok(text) = fs::read_to_string(path) {
                if let Some(style) = parse_pydoclint_style(&text) {
                    formatter_docstring_style = style;
                }
            }
        }
        if let Some(style) = vscode.pydoclint_overrides.style {
            formatter_docstring_style = style;
        }

        // Priority model:
        // 1. VSCode settings are a fallback.
        // 2. If pyproject.toml is found, it replaces VSCode fallback for project-level config.
        // 3. File comments are a final local layer for the current file.
        let mut select = if has_product_pyproject {
            pyproject.select.clone()
        } else {
            vscode.select.clone()
        };
        select.extend(inline.select.iter().cloned());

        let mut ignore = if has_product_pyproject {
            pyproject.ignore.clone()
        } else {
            vscode.ignore.clone()
        };
        ignore.extend(inline.ignore.iter().cloned());

        let active_codes = active_codes(strict, &select, &ignore);
        Self {
            strict,
            select,
            ignore,
            active_codes,
            pyproject_path: pyproject.found_path.clone(),
            formatter_docstring_style,
            pydoclint_config_path: vscode.pydoclint_config_path.clone(),
            pydoclint_inferred_config_context: vscode.pydoclint_inferred_config_context.clone(),
            pydoclint_overrides: vscode.pydoclint_overrides.clone(),
            assertion_helpers: if has_product_pyproject {
                pyproject.assertion_helpers.clone()
            } else {
                Vec::new()
            },
            exception_boundary_functions: if has_product_pyproject {
                pyproject.exception_boundary_functions.clone()
            } else {
                Vec::new()
            },
        }
    }

    pub fn is_enabled(&self, code: &str) -> bool {
        self.active_codes.iter().any(|enabled| enabled == code)
    }
}

fn active_codes(strict: bool, select: &[String], ignore: &[String]) -> Vec<String> {
    let mut enabled = Vec::new();

    for rule in ALL_RULES {
        if rule.level == RuleLevel::Normal || (strict && rule.level == RuleLevel::Strict) {
            enabled.push(rule.code.to_string());
        }
    }

    for selector in select {
        for rule in ALL_RULES {
            if code_matches_selector(rule.code, selector) && !enabled.iter().any(|c| c == rule.code)
            {
                enabled.push(rule.code.to_string());
            }
        }
    }

    enabled.retain(|code| {
        !ignore
            .iter()
            .any(|selector| code_matches_selector(code, selector))
    });
    enabled.sort();
    enabled
}

pub fn load_pyproject_for_file(file_path: &Path) -> PyProjectConfig {
    let mut dir = if file_path.is_dir() {
        file_path.to_path_buf()
    } else {
        file_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };

    loop {
        let candidate = dir.join("pyproject.toml");
        if candidate.is_file() {
            let text = fs::read_to_string(&candidate).unwrap_or_default();
            if pyproject_contains_relevant_section(&text) {
                let mut config = parse_pyproject_toml(&text);
                config.found_path = Some(candidate);
                return config;
            }
        }
        if !dir.pop() {
            break;
        }
    }

    PyProjectConfig::default()
}

fn pyproject_contains_relevant_section(text: &str) -> bool {
    text.lines().any(|raw_line| {
        matches!(
            strip_toml_comment(raw_line).trim(),
            "[tool.sklint]" | "[tool.pydoclint]" | "[tool.sklint.pydoclint]"
        )
    })
}

pub fn parse_pyproject_toml(text: &str) -> PyProjectConfig {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Section {
        None,
        Sklint,
        Pydoclint,
        SklintPydoclint,
    }

    let mut section = Section::None;
    let mut config = PyProjectConfig::default();
    let mut upstream_pydoclint_style = None;
    let mut sklint_pydoclint_style = None;

    for line_without_comment in logical_toml_lines(text) {
        if line_without_comment.starts_with('[') && line_without_comment.ends_with(']') {
            section = match line_without_comment.as_str() {
                "[tool.sklint]" => {
                    config.has_sklint_section = true;
                    Section::Sklint
                }
                "[tool.pydoclint]" => Section::Pydoclint,
                "[tool.sklint.pydoclint]" => Section::SklintPydoclint,
                _ => Section::None,
            };
            continue;
        }

        let Some((key, value)) = line_without_comment.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match section {
            Section::Sklint => match key {
                "strict" => match parse_bool(value) {
                    Some(parsed) => config.strict = Some(parsed),
                    None => config.errors.push(format!(
                        "[tool.sklint] strict must be a TOML boolean, got `{value}`"
                    )),
                },
                "select" => match parse_string_array_checked(value) {
                    Ok(parsed) => config.select = parsed,
                    Err(message) => config
                        .errors
                        .push(format!("[tool.sklint] select {message}")),
                },
                "ignore" => match parse_string_array_checked(value) {
                    Ok(parsed) => config.ignore = parsed,
                    Err(message) => config
                        .errors
                        .push(format!("[tool.sklint] ignore {message}")),
                },
                "assertion_helpers" | "assertion-helpers" => {
                    match parse_string_array_preserving_case(value) {
                        Ok(parsed) => config.assertion_helpers = parsed,
                        Err(message) => config
                            .errors
                            .push(format!("[tool.sklint] {key} {message}")),
                    }
                }
                "exception_boundary_functions" | "exception-boundary-functions" => {
                    match parse_string_array_preserving_case(value) {
                        Ok(parsed) => config.exception_boundary_functions = parsed,
                        Err(message) => config
                            .errors
                            .push(format!("[tool.sklint] {key} {message}")),
                    }
                }
                "formatter-docstring-style" | "formatter_docstring_style" => {
                    match DocStyle::parse(trim_toml_string(value)) {
                        Some(style) if is_quoted_toml_string(value) => {
                            config.formatter_docstring_style = Some(style);
                        }
                        _ => config.errors.push(format!(
                            "[tool.sklint] {key} must be one of 'google', 'numpy', 'sphinx' as a TOML string, got `{value}`"
                        )),
                    }
                }
                "style" => match DocStyle::parse(trim_toml_string(value)) {
                    Some(style) if is_quoted_toml_string(value) => {
                        sklint_pydoclint_style = Some(style);
                    }
                    _ => config.errors.push(format!(
                        "[tool.sklint] style must be one of 'google', 'numpy', 'sphinx' as a TOML string, got `{value}`"
                    )),
                },
                _ => {}
            },
            Section::Pydoclint if key.replace('-', "_").eq_ignore_ascii_case("style") => {
                upstream_pydoclint_style = DocStyle::parse(trim_toml_string(value));
            }
            Section::SklintPydoclint if key.replace('-', "_").eq_ignore_ascii_case("style") => {
                sklint_pydoclint_style = DocStyle::parse(trim_toml_string(value));
            }
            _ => {}
        }
    }

    config.pydoclint_style = sklint_pydoclint_style.or(upstream_pydoclint_style);
    normalize_code_list(&mut config.select);
    normalize_code_list(&mut config.ignore);
    normalize_pattern_list(&mut config.assertion_helpers);
    normalize_pattern_list(&mut config.exception_boundary_functions);
    validate_selectors("select", &config.select, &mut config.errors);
    validate_selectors("ignore", &config.ignore, &mut config.errors);
    config.errors.sort();
    config.errors.dedup();
    config
}

fn parse_pydoclint_style(text: &str) -> Option<DocStyle> {
    parse_pyproject_toml(text).pydoclint_style
}

pub fn parse_inline_config(source: &str) -> FileInlineConfig {
    let mut config = FileInlineConfig::default();

    for line in source.lines().take(20) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed.starts_with('#') {
            break;
        }

        let Some(directive) = sklint_directive(trimmed) else {
            continue;
        };

        let lower = directive.to_ascii_lowercase();
        if lower == "strict" {
            config.strict = Some(true);
            continue;
        }
        if lower == "non-strict" || lower == "nonstrict" || lower == "strict=false" {
            config.strict = Some(false);
            continue;
        }

        for part in directive.split(';') {
            let part = part.trim();
            let lower_part = part.to_ascii_lowercase();
            if lower_part.starts_with("select=") {
                config
                    .select
                    .extend(parse_csv_codes(&part["select=".len()..]));
            } else if lower_part.starts_with("ignore=") {
                config
                    .ignore
                    .extend(parse_csv_codes(&part["ignore=".len()..]));
            }
        }
    }

    normalize_code_list(&mut config.select);
    normalize_code_list(&mut config.ignore);
    config
}

pub fn sklint_directive(comment_line: &str) -> Option<&str> {
    let trimmed = comment_line.trim_start();
    let after_hash = trimmed.strip_prefix('#')?.trim_start();
    let after_name = after_hash.strip_prefix("sklint")?.trim_start();
    let directive = after_name.strip_prefix(':')?.trim_start();
    Some(directive)
}

pub fn parse_csv_codes(text: &str) -> Vec<String> {
    text.split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .map(|item| {
            item.trim()
                .trim_matches(|c| c == '[' || c == ']' || c == '"' || c == '\'')
        })
        .filter(|item| !item.is_empty())
        .map(|item| item.to_ascii_uppercase())
        .collect()
}

fn is_quoted_toml_string(value: &str) -> bool {
    let value = value.trim();
    value.len() >= 2
        && ((value.starts_with('\'') && value.ends_with('\''))
            || (value.starts_with('"') && value.ends_with('"')))
}

fn trim_toml_string(value: &str) -> &str {
    value.trim().trim_matches(|c| c == '"' || c == '\'')
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_string_array_checked(value: &str) -> Result<Vec<String>, String> {
    parse_string_array(value, true)
}

fn parse_string_array_preserving_case(value: &str) -> Result<Vec<String>, String> {
    parse_string_array(value, false)
}

fn parse_string_array(value: &str, uppercase: bool) -> Result<Vec<String>, String> {
    let value = value.trim();
    let Some(inner) = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    else {
        return Err(format!("must be an array of strings, got `{value}`"));
    };

    let bytes = inner.as_bytes();
    let mut index = 0usize;
    let mut items = Vec::new();
    while index < bytes.len() {
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b',') {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let quote = bytes[index];
        if !matches!(quote, b'\'' | b'"') {
            return Err(format!("must contain only quoted strings, got `{value}`"));
        }
        index += 1;
        let start = index;
        let mut escaped = false;
        while index < bytes.len() {
            let byte = bytes[index];
            if quote == b'"' && escaped {
                escaped = false;
                index += 1;
                continue;
            }
            if quote == b'"' && byte == b'\\' {
                escaped = true;
                index += 1;
                continue;
            }
            if byte == quote {
                break;
            }
            index += 1;
        }
        if index >= bytes.len() {
            return Err(format!("contains an unterminated string in `{value}`"));
        }
        let item = inner[start..index].trim();
        if item.is_empty() {
            return Err("must not contain empty selectors".to_string());
        }
        items.push(if uppercase {
            item.to_ascii_uppercase()
        } else {
            item.to_string()
        });
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index < bytes.len() && bytes[index] != b',' {
            return Err(format!("must separate strings with commas, got `{value}`"));
        }
    }
    Ok(items)
}

pub fn validate_selector(selector: &str) -> bool {
    let selector = selector.trim().to_ascii_uppercase();
    selector == "ALL"
        || ALL_RULES
            .iter()
            .any(|rule| code_matches_selector(rule.code, &selector))
}

fn validate_selectors(kind: &str, selectors: &[String], errors: &mut Vec<String>) {
    for selector in selectors {
        if !validate_selector(selector) {
            errors.push(format!(
                "[tool.sklint] {kind} contains unknown selector `{selector}`"
            ));
        }
    }
}

fn strip_toml_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == Some('"') && ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if matches!(ch, '\'' | '"') => quote = Some(ch),
            None if ch == '#' => return &line[..idx],
            None => {}
        }
    }
    line
}

fn logical_toml_lines(text: &str) -> Vec<String> {
    let raw_lines = text.lines().collect::<Vec<_>>();
    let mut out = Vec::new();
    let mut index = 0usize;

    while index < raw_lines.len() {
        let first = strip_toml_comment(raw_lines[index]).trim();
        index += 1;
        if first.is_empty() {
            continue;
        }

        let mut logical = first.to_string();
        let starts_multiline_array = first
            .split_once('=')
            .map(|(_, value)| value.trim())
            .is_some_and(|value| value.starts_with('[') && !toml_array_is_complete(value));
        if starts_multiline_array {
            while index < raw_lines.len() {
                let continuation = strip_toml_comment(raw_lines[index]).trim();
                index += 1;
                if !continuation.is_empty() {
                    logical.push(' ');
                    logical.push_str(continuation);
                }
                let current_value = logical
                    .split_once('=')
                    .map(|(_, value)| value.trim())
                    .unwrap_or("");
                if toml_array_is_complete(current_value) {
                    break;
                }
            }
        }
        out.push(logical);
    }

    out
}

fn toml_array_is_complete(value: &str) -> bool {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == Some('"') && ch == '\\' {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if matches!(ch, '\'' | '"') => quote = Some(ch),
            None if ch == '[' => depth += 1,
            None if ch == ']' => depth = depth.saturating_sub(1),
            None => {}
        }
    }
    depth == 0 && quote.is_none()
}

fn normalize_code_list(list: &mut Vec<String>) {
    for code in list.iter_mut() {
        *code = code.trim().to_ascii_uppercase();
    }
    list.retain(|code| !code.is_empty());
    list.sort();
    list.dedup();
}

fn normalize_pattern_list(list: &mut Vec<String>) {
    for pattern in list.iter_mut() {
        *pattern = pattern.trim().to_string();
    }
    list.retain(|pattern| !pattern.is_empty());
    list.sort();
    list.dedup();
}

pub(crate) fn name_matches_pattern(name: &str, pattern: &str) -> bool {
    if !pattern.contains('*') {
        return name == pattern;
    }

    let starts_with_wildcard = pattern.starts_with('*');
    let ends_with_wildcard = pattern.ends_with('*');
    let parts = pattern
        .split('*')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return true;
    }

    let mut search_from = 0usize;
    for (index, part) in parts.iter().enumerate() {
        let Some(relative) = name[search_from..].find(part) else {
            return false;
        };
        let found_at = search_from + relative;
        if index == 0 && !starts_with_wildcard && found_at != 0 {
            return false;
        }
        search_from = found_at + part.len();
    }

    ends_with_wildcard || search_from == name.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pyproject_parses_tool_section() {
        let parsed = parse_pyproject_toml(
            r#"
[project]
name = "demo"

[tool.sklint]
strict = true
select = ["SK101"]
ignore = ["SK001"]
"#,
        );
        assert_eq!(parsed.strict, Some(true));
        assert_eq!(parsed.select, vec!["SK101"]);
        assert_eq!(parsed.ignore, vec!["SK001"]);
    }

    #[test]
    fn pyproject_parses_multiline_select_and_ignore_arrays() {
        let parsed = parse_pyproject_toml(
            r#"
[tool.sklint]
strict = false
select = [
    "SK901", # comparison policy
    'SKD301',
]
ignore = [
    "SK201",
    'SK001', # trailing comma is valid TOML
]
"#,
        );
        assert_eq!(parsed.select, vec!["SK901", "SKD301"]);
        assert_eq!(parsed.ignore, vec!["SK001", "SK201"]);
    }

    #[test]
    fn pyproject_parses_semantic_helper_patterns_without_uppercasing() {
        let parsed = parse_pyproject_toml(
            r#"
[tool.sklint]
assertion_helpers = ["verify", "verify_*", "assert_*"]
exception_boundary_functions = ["close", "shutdown", "_rollback_*"]
"#,
        );
        assert_eq!(
            parsed.assertion_helpers,
            vec!["assert_*", "verify", "verify_*"]
        );
        assert_eq!(
            parsed.exception_boundary_functions,
            vec!["_rollback_*", "close", "shutdown"]
        );
        assert!(parsed.errors.is_empty());
    }

    #[test]
    fn invalid_semantic_helper_patterns_are_configuration_errors() {
        let parsed = parse_pyproject_toml(
            "[tool.sklint]\nassertion_helpers = [123]\nexception_boundary_functions = false\n",
        );
        assert!(parsed
            .errors
            .iter()
            .any(|message| message.contains("assertion_helpers")));
        assert!(parsed
            .errors
            .iter()
            .any(|message| message.contains("exception_boundary_functions")));
    }

    #[test]
    fn semantic_name_patterns_use_simple_anchored_glob_matching() {
        assert!(name_matches_pattern("verify_equal", "verify_*"));
        assert!(name_matches_pattern("assert_called_once_with", "assert_*"));
        assert!(name_matches_pattern(
            "_rollback_native_state",
            "_rollback_*"
        ));
        assert!(name_matches_pattern(
            "prefix_middle_suffix",
            "prefix_*_suffix"
        ));
        assert!(name_matches_pattern("pkg.verify_equal", "*.verify_*"));
        assert!(!name_matches_pattern("my_verify_equal", "verify_*"));
        assert!(!name_matches_pattern("verify_equal_extra", "verify_equal"));
    }

    #[test]
    fn toml_comments_inside_strings_are_preserved() {
        assert_eq!(
            strip_toml_comment("select = ['SK901#x'] # comment"),
            "select = ['SK901#x'] "
        );
    }

    #[test]
    fn invalid_formatter_style_is_reported_instead_of_defaulting_silently() {
        let parsed = parse_pyproject_toml("[tool.sklint]\nformatter-docstring-style = \"rest\"\n");
        assert!(parsed.formatter_docstring_style.is_none());
        assert!(parsed
            .errors
            .iter()
            .any(|message| message.contains("formatter-docstring-style")));
    }

    #[test]
    fn invalid_sklint_selector_is_reported_instead_of_failing_open() {
        let parsed = parse_pyproject_toml("[tool.sklint]\nstrict = false\nselect = [\"SK999\"]\n");
        assert!(parsed.select.contains(&"SK999".to_string()));
        assert!(parsed
            .errors
            .iter()
            .any(|message| message.contains("SK999")));
    }

    #[test]
    fn non_string_selector_array_is_reported() {
        let parsed = parse_pyproject_toml("[tool.sklint]\nselect = [123]\n");
        assert!(parsed.select.is_empty());
        assert!(parsed
            .errors
            .iter()
            .any(|message| message.contains("quoted strings")));
    }

    #[test]
    fn invalid_strict_type_is_reported() {
        let parsed = parse_pyproject_toml("[tool.sklint]\nstrict = \"true\"\n");
        assert!(parsed
            .errors
            .iter()
            .any(|message| message.contains("TOML boolean")));
    }

    #[test]
    fn skd301_is_opt_in_even_in_strict_mode() {
        let config = EffectiveConfig::resolve(
            &VscodeConfig::default(),
            &PyProjectConfig {
                found_path: Some(PathBuf::from("pyproject.toml")),
                has_sklint_section: true,
                strict: Some(true),
                ..PyProjectConfig::default()
            },
            &FileInlineConfig::default(),
        );
        assert!(!config.is_enabled("SKD301"));
    }

    #[test]
    fn skd301_can_be_enabled_explicitly_via_doc_alias() {
        let config = EffectiveConfig::resolve(
            &VscodeConfig::default(),
            &PyProjectConfig {
                found_path: Some(PathBuf::from("pyproject.toml")),
                has_sklint_section: true,
                strict: Some(false),
                select: vec!["DOC301".into()],
                ..PyProjectConfig::default()
            },
            &FileInlineConfig::default(),
        );
        assert!(config.is_enabled("SKD301"));
    }

    #[test]
    fn formatter_docstring_style_defaults_to_google() {
        let config = EffectiveConfig::resolve(
            &VscodeConfig::default(),
            &PyProjectConfig::default(),
            &FileInlineConfig::default(),
        );
        assert_eq!(config.formatter_docstring_style, DocStyle::Google);
    }

    #[test]
    fn pyproject_formatter_docstring_style_overrides_vscode_fallback() {
        let config = EffectiveConfig::resolve(
            &VscodeConfig {
                formatter_docstring_style: Some(DocStyle::Sphinx),
                ..VscodeConfig::default()
            },
            &PyProjectConfig {
                found_path: Some(PathBuf::from("pyproject.toml")),
                has_sklint_section: true,
                formatter_docstring_style: Some(DocStyle::Numpy),
                ..PyProjectConfig::default()
            },
            &FileInlineConfig::default(),
        );
        assert_eq!(config.formatter_docstring_style, DocStyle::Numpy);
    }

    #[test]
    fn syntax_validity_gate_is_active_without_strict_project_config() {
        let effective = EffectiveConfig::resolve(
            &VscodeConfig::default(),
            &PyProjectConfig::default(),
            &FileInlineConfig::default(),
        );
        assert!(effective.is_enabled("SKD002"));
        assert!(effective.is_enabled("SK903"));
    }

    #[test]
    fn strict_enables_strict_rules() {
        let config = EffectiveConfig::resolve(
            &VscodeConfig::default(),
            &PyProjectConfig {
                found_path: Some(PathBuf::from("pyproject.toml")),
                has_sklint_section: true,
                strict: Some(true),
                ..PyProjectConfig::default()
            },
            &FileInlineConfig::default(),
        );
        assert!(config.is_enabled("SK101"));
    }

    #[test]
    fn pyproject_without_sklint_section_is_ignored() {
        let temp = std::env::temp_dir().join("sklint-no-tool-section");
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(temp.join("pkg")).unwrap();
        std::fs::write(
            temp.join("pyproject.toml"),
            "[project]
name = \"example\"
version = \"0.1.0\"
",
        )
        .unwrap();
        let config = load_pyproject_for_file(&temp.join("pkg/example.py"));
        assert!(config.found_path.is_none());
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn pyproject_replaces_vscode_fallback() {
        let config = EffectiveConfig::resolve(
            &VscodeConfig {
                strict: Some(true),
                select: vec!["SK101".into()],
                ..VscodeConfig::default()
            },
            &PyProjectConfig {
                found_path: Some(PathBuf::from("pyproject.toml")),
                has_sklint_section: true,
                strict: Some(false),
                select: Vec::new(),
                ignore: Vec::new(),
                ..PyProjectConfig::default()
            },
            &FileInlineConfig::default(),
        );
        assert!(!config.strict);
        assert!(!config.is_enabled("SK101"));
    }

    #[test]
    fn sklint_pydoclint_style_has_fixed_priority_independent_of_section_order() {
        for text in [
            "[tool.sklint.pydoclint]\nstyle = 'sphinx'\n[tool.pydoclint]\nstyle = 'numpy'\n",
            "[tool.pydoclint]\nstyle = 'numpy'\n[tool.sklint.pydoclint]\nstyle = 'sphinx'\n",
        ] {
            let parsed = parse_pyproject_toml(text);
            assert_eq!(parsed.pydoclint_style, Some(DocStyle::Sphinx));
        }
    }

    #[test]
    fn pydoclint_only_pyproject_keeps_vscode_product_settings_but_overrides_style() {
        let config = EffectiveConfig::resolve(
            &VscodeConfig {
                strict: Some(true),
                formatter_docstring_style: Some(DocStyle::Google),
                ..VscodeConfig::default()
            },
            &PyProjectConfig {
                found_path: Some(PathBuf::from("pyproject.toml")),
                has_sklint_section: false,
                pydoclint_style: Some(DocStyle::Numpy),
                ..PyProjectConfig::default()
            },
            &FileInlineConfig::default(),
        );
        assert!(config.strict);
        assert_eq!(config.formatter_docstring_style, DocStyle::Numpy);
    }

    #[test]
    fn inferred_pydoclint_context_controls_shared_formatter_style() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sklint-config-common-{unique}"));
        let child = root.join("child");
        fs::create_dir_all(&child).expect("create nested project");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.pydoclint]\nstyle = 'google'\n",
        )
        .expect("write common config");
        fs::write(
            child.join("pyproject.toml"),
            "[tool.pydoclint]\nstyle = 'sphinx'\n",
        )
        .expect("write child config");
        let file = child.join("case.py");
        fs::write(&file, "x = 1\n").expect("write source");
        let per_file_project = load_pyproject_for_file(&file);

        let config = EffectiveConfig::resolve(
            &VscodeConfig {
                pydoclint_inferred_config_context: Some(root.clone()),
                ..VscodeConfig::default()
            },
            &per_file_project,
            &FileInlineConfig::default(),
        );
        assert_eq!(config.formatter_docstring_style, DocStyle::Google);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cli_pydoclint_style_overrides_project_and_formatter_style() {
        let config = EffectiveConfig::resolve(
            &VscodeConfig {
                formatter_docstring_style: Some(DocStyle::Google),
                pydoclint_overrides: PydoclintCliOverrides {
                    style: Some(DocStyle::Sphinx),
                    ..PydoclintCliOverrides::default()
                },
                ..VscodeConfig::default()
            },
            &PyProjectConfig {
                found_path: Some(PathBuf::from("pyproject.toml")),
                has_sklint_section: true,
                formatter_docstring_style: Some(DocStyle::Numpy),
                pydoclint_style: Some(DocStyle::Google),
                ..PyProjectConfig::default()
            },
            &FileInlineConfig::default(),
        );
        assert_eq!(config.formatter_docstring_style, DocStyle::Sphinx);
    }

    #[test]
    fn inline_ignore_wins_last() {
        let config = EffectiveConfig::resolve(
            &VscodeConfig {
                strict: Some(true),
                ..VscodeConfig::default()
            },
            &PyProjectConfig::default(),
            &FileInlineConfig {
                ignore: vec!["SK101".into()],
                ..FileInlineConfig::default()
            },
        );
        assert!(!config.is_enabled("SK101"));
    }
}
