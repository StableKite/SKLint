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

    for raw_line in text.lines() {
        let line_without_comment = strip_toml_comment(raw_line).trim().to_string();
        if line_without_comment.is_empty() {
            continue;
        }

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
                "strict" => config.strict = parse_bool(value),
                "select" => config.select = parse_string_array(value),
                "ignore" => config.ignore = parse_string_array(value),
                "formatter-docstring-style" | "formatter_docstring_style" => {
                    config.formatter_docstring_style = DocStyle::parse(trim_toml_string(value));
                }
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

fn parse_string_array(value: &str) -> Vec<String> {
    let value = value.trim();
    let value = value.strip_prefix('[').unwrap_or(value);
    let value = value.strip_suffix(']').unwrap_or(value);
    value
        .split(',')
        .map(|item| item.trim().trim_matches(|c| c == '"' || c == '\''))
        .filter(|item| !item.is_empty())
        .map(|item| item.to_ascii_uppercase())
        .collect()
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if ch == '#' && !in_string {
            return &line[..idx];
        }
    }
    line
}

fn normalize_code_list(list: &mut Vec<String>) {
    for code in list.iter_mut() {
        *code = code.trim().to_ascii_uppercase();
    }
    list.retain(|code| !code.is_empty());
    list.sort();
    list.dedup();
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
