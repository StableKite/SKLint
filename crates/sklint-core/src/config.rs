use crate::rules::{code_matches_selector, RuleLevel, ALL_RULES};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VscodeConfig {
    pub strict: Option<bool>,
    pub select: Vec<String>,
    pub ignore: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PyProjectConfig {
    pub found_path: Option<PathBuf>,
    pub strict: Option<bool>,
    pub select: Vec<String>,
    pub ignore: Vec<String>,
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
}

impl EffectiveConfig {
    pub fn resolve(
        vscode: &VscodeConfig,
        pyproject: &PyProjectConfig,
        inline: &FileInlineConfig,
    ) -> Self {
        let has_pyproject = pyproject.found_path.is_some();
        let base_strict = if has_pyproject {
            pyproject.strict.unwrap_or(false)
        } else {
            vscode.strict.unwrap_or(false)
        };
        let strict = inline.strict.unwrap_or(base_strict);

        // Priority model:
        // 1. VSCode settings are a fallback.
        // 2. If pyproject.toml is found, it replaces VSCode fallback for project-level config.
        // 3. File comments are a final local layer for the current file.
        let mut select = if has_pyproject {
            pyproject.select.clone()
        } else {
            vscode.select.clone()
        };
        select.extend(inline.select.iter().cloned());

        let mut ignore = if has_pyproject {
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
            if pyproject_contains_sklint_section(&text) {
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

fn pyproject_contains_sklint_section(text: &str) -> bool {
    text.lines().any(|raw_line| {
        let line_without_comment = strip_toml_comment(raw_line).trim();
        line_without_comment == "[tool.sklint]"
    })
}

pub fn parse_pyproject_toml(text: &str) -> PyProjectConfig {
    let mut in_section = false;
    let mut config = PyProjectConfig::default();

    for raw_line in text.lines() {
        let line_without_comment = strip_toml_comment(raw_line).trim().to_string();
        if line_without_comment.is_empty() {
            continue;
        }

        if line_without_comment.starts_with('[') && line_without_comment.ends_with(']') {
            in_section = line_without_comment == "[tool.sklint]";
            continue;
        }

        if !in_section {
            continue;
        }

        if let Some((key, value)) = line_without_comment.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "strict" => config.strict = parse_bool(value),
                "select" => config.select = parse_string_array(value),
                "ignore" => config.ignore = parse_string_array(value),
                _ => {}
            }
        }
    }

    normalize_code_list(&mut config.select);
    normalize_code_list(&mut config.ignore);
    config
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
    fn strict_enables_strict_rules() {
        let config = EffectiveConfig::resolve(
            &VscodeConfig::default(),
            &PyProjectConfig {
                found_path: Some(PathBuf::from("pyproject.toml")),
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
                strict: Some(false),
                select: Vec::new(),
                ignore: Vec::new(),
            },
            &FileInlineConfig::default(),
        );
        assert!(!config.strict);
        assert!(!config.is_enabled("SK101"));
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
