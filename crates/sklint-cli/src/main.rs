use sklint_core::{
    analyze, formatter::format_source, rules::ALL_RULES, AnalysisInput, Diagnostic, VscodeConfig,
};
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process;

#[derive(Debug, Clone, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone)]
struct CheckArgs {
    format: OutputFormat,
    fix: bool,
    stdin_filename: Option<PathBuf>,
    vscode_config: VscodeConfig,
    paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct FormatArgs {
    check: bool,
    stdin_filename: Option<PathBuf>,
    vscode_config: VscodeConfig,
    paths: Vec<PathBuf>,
}

fn main() {
    let code = match run() {
        Ok(exit_code) => exit_code,
        Err(message) => {
            eprintln!("sklint: {message}");
            2
        }
    };
    process::exit(code);
}

fn run() -> Result<i32, String> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("check") => run_check(parse_check_args(args.collect())?),
        Some("format") => run_format(parse_format_args(args.collect())?),
        Some("rules") => {
            print_rules();
            Ok(0)
        }
        Some("explain") => {
            let code = args
                .next()
                .ok_or_else(|| "usage: sklint explain SK001".to_string())?;
            explain(&code);
            Ok(0)
        }
        Some("--version") | Some("-V") => {
            println!("sklint {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Some("--help") | Some("-h") | None => {
            print_help();
            Ok(0)
        }
        Some(other) => Err(format!("unknown command `{other}`. Use `sklint --help`.")),
    }
}

fn parse_check_args(raw: Vec<String>) -> Result<CheckArgs, String> {
    let mut format = OutputFormat::Text;
    let mut fix = false;
    let mut stdin_filename = None;
    let mut vscode_config = VscodeConfig::default();
    let mut paths = Vec::new();

    let mut idx = 0;
    while idx < raw.len() {
        match raw[idx].as_str() {
            "--format" => {
                idx += 1;
                let value = raw
                    .get(idx)
                    .ok_or_else(|| "--format requires value".to_string())?;
                format = parse_output_format(value)?;
            }
            "--fix" => fix = true,
            "--stdin-filename" => {
                idx += 1;
                let value = raw
                    .get(idx)
                    .ok_or_else(|| "--stdin-filename requires value".to_string())?;
                stdin_filename = Some(PathBuf::from(value));
            }
            "--vscode-strict" => {
                idx += 1;
                parse_vscode_strict(&raw, idx, &mut vscode_config)?;
            }
            "--vscode-select" => {
                idx += 1;
                let value = raw
                    .get(idx)
                    .ok_or_else(|| "--vscode-select requires comma-separated value".to_string())?;
                vscode_config.select = split_codes(value);
            }
            "--vscode-ignore" => {
                idx += 1;
                let value = raw
                    .get(idx)
                    .ok_or_else(|| "--vscode-ignore requires comma-separated value".to_string())?;
                vscode_config.ignore = split_codes(value);
            }
            "-" => paths.push(PathBuf::from("-")),
            other if other.starts_with('-') => return Err(format!("unknown option `{other}`")),
            other => paths.push(PathBuf::from(other)),
        }
        idx += 1;
    }

    if paths.is_empty() {
        paths.push(PathBuf::from("."));
    }

    Ok(CheckArgs {
        format,
        fix,
        stdin_filename,
        vscode_config,
        paths,
    })
}

fn parse_format_args(raw: Vec<String>) -> Result<FormatArgs, String> {
    let mut check = false;
    let mut stdin_filename = None;
    let mut vscode_config = VscodeConfig::default();
    let mut paths = Vec::new();

    let mut idx = 0;
    while idx < raw.len() {
        match raw[idx].as_str() {
            "--check" => check = true,
            "--stdin-filename" => {
                idx += 1;
                let value = raw
                    .get(idx)
                    .ok_or_else(|| "--stdin-filename requires value".to_string())?;
                stdin_filename = Some(PathBuf::from(value));
            }
            "--vscode-strict" => {
                idx += 1;
                parse_vscode_strict(&raw, idx, &mut vscode_config)?;
            }
            "--vscode-select" => {
                idx += 1;
                let value = raw
                    .get(idx)
                    .ok_or_else(|| "--vscode-select requires comma-separated value".to_string())?;
                vscode_config.select = split_codes(value);
            }
            "--vscode-ignore" => {
                idx += 1;
                let value = raw
                    .get(idx)
                    .ok_or_else(|| "--vscode-ignore requires comma-separated value".to_string())?;
                vscode_config.ignore = split_codes(value);
            }
            "-" => paths.push(PathBuf::from("-")),
            other if other.starts_with('-') => return Err(format!("unknown option `{other}`")),
            other => paths.push(PathBuf::from(other)),
        }
        idx += 1;
    }

    if paths.is_empty() {
        paths.push(PathBuf::from("."));
    }

    Ok(FormatArgs {
        check,
        stdin_filename,
        vscode_config,
        paths,
    })
}

fn parse_output_format(value: &str) -> Result<OutputFormat, String> {
    match value {
        "json" => Ok(OutputFormat::Json),
        "text" => Ok(OutputFormat::Text),
        other => Err(format!("unknown format `{other}`")),
    }
}

fn parse_vscode_strict(
    raw: &[String],
    idx: usize,
    vscode_config: &mut VscodeConfig,
) -> Result<(), String> {
    let value = raw
        .get(idx)
        .ok_or_else(|| "--vscode-strict requires true or false".to_string())?;
    vscode_config.strict = match value.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => return Err("--vscode-strict requires true or false".to_string()),
    };
    Ok(())
}

fn run_check(args: CheckArgs) -> Result<i32, String> {
    if args.fix {
        run_format(FormatArgs {
            check: false,
            stdin_filename: args.stdin_filename.clone(),
            vscode_config: args.vscode_config.clone(),
            paths: args.paths.clone(),
        })?;
    }

    let mut diagnostics = Vec::new();
    for path in &args.paths {
        if path == Path::new("-") {
            let mut source = String::new();
            io::stdin()
                .read_to_string(&mut source)
                .map_err(|err| err.to_string())?;
            let display_path = args
                .stdin_filename
                .clone()
                .unwrap_or_else(|| PathBuf::from("<stdin>"));
            let report = analyze(AnalysisInput {
                path: display_path,
                source,
                vscode_config: args.vscode_config.clone(),
            });
            diagnostics.extend(report.diagnostics);
            continue;
        }

        for file in files_for_path(path)? {
            let source =
                fs::read_to_string(&file).map_err(|err| format!("{}: {err}", file.display()))?;
            let report = analyze(AnalysisInput {
                path: file,
                source,
                vscode_config: args.vscode_config.clone(),
            });
            diagnostics.extend(report.diagnostics);
        }
    }

    diagnostics.sort_by(|a, b| {
        (a.path.as_str(), a.line, a.column, a.code.as_str()).cmp(&(
            b.path.as_str(),
            b.line,
            b.column,
            b.code.as_str(),
        ))
    });

    match args.format {
        OutputFormat::Text => print_text(&diagnostics),
        OutputFormat::Json => print_json(&diagnostics),
    }

    Ok(if diagnostics.is_empty() { 0 } else { 1 })
}

fn run_format(args: FormatArgs) -> Result<i32, String> {
    let mut changed = Vec::new();

    for path in &args.paths {
        if path == Path::new("-") {
            let mut source = String::new();
            io::stdin()
                .read_to_string(&mut source)
                .map_err(|err| err.to_string())?;
            let display_path = args
                .stdin_filename
                .clone()
                .unwrap_or_else(|| PathBuf::from("<stdin>"));
            let report = format_source(display_path, source, args.vscode_config.clone());
            print!("{}", report.source);
            return Ok(0);
        }

        for file in files_for_path(path)? {
            let source =
                fs::read_to_string(&file).map_err(|err| format!("{}: {err}", file.display()))?;
            let report = format_source(file.clone(), source.clone(), args.vscode_config.clone());
            if report.source != source {
                changed.push(file.clone());
                if !args.check {
                    fs::write(&file, report.source)
                        .map_err(|err| format!("{}: {err}", file.display()))?;
                }
            }
        }
    }

    if args.check {
        for file in &changed {
            println!("{} needs formatting", file.display());
        }
        Ok(if changed.is_empty() { 0 } else { 1 })
    } else {
        for file in &changed {
            println!("{} formatted", file.display());
        }
        Ok(0)
    }
}

fn files_for_path(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_dir() {
        collect_python_files(path)
    } else if is_python_file(path) {
        Ok(vec![path.to_path_buf()])
    } else {
        Ok(Vec::new())
    }
}

fn collect_python_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|err| err.to_string())?;
            let path = entry.path();
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if file_name.starts_with('.') || file_name == "target" || file_name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if is_python_file(&path) {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn is_python_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("py") | Some("pyi")
    )
}

fn print_text(diagnostics: &[Diagnostic]) {
    for diag in diagnostics {
        println!(
            "{}:{}:{}: {} {}",
            diag.path, diag.line, diag.column, diag.code, diag.message
        );
    }
}

fn print_json(diagnostics: &[Diagnostic]) {
    print!("{{\"diagnostics\":[");
    for (idx, diag) in diagnostics.iter().enumerate() {
        if idx > 0 {
            print!(",");
        }
        print!(
            "{{\"code\":\"{}\",\"message\":\"{}\",\"path\":\"{}\",\"line\":{},\"column\":{},\"end_line\":{},\"end_column\":{},\"level\":\"{}\"",
            json_escape(&diag.code),
            json_escape(&diag.message),
            json_escape(&diag.path),
            diag.line,
            diag.column,
            diag.end_line,
            diag.end_column,
            json_escape(&diag.level),
        );
        if let Some(line) = diag.suppression_line {
            print!(",\"suppression_line\":{}", line);
        }
        if let Some(fix) = &diag.fix {
            print!(
                ",\"fix\":{{\"message\":\"{}\",\"replacement\":\"{}\",\"start_line\":{},\"start_column\":{},\"end_line\":{},\"end_column\":{}}}",
                json_escape(&fix.message),
                json_escape(&fix.replacement),
                fix.start_line,
                fix.start_column,
                fix.end_line,
                fix.end_column,
            );
        }
        print!("}}");
    }
    println!("]}}");
}

fn json_escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

fn split_codes(text: &str) -> Vec<String> {
    text.split(',')
        .map(|item| item.trim().to_ascii_uppercase())
        .filter(|item| !item.is_empty())
        .collect()
}

fn print_rules() {
    for rule in ALL_RULES {
        println!("{}\t{:?}\t{}", rule.code, rule.level, rule.summary);
    }
}

fn explain(code: &str) {
    let code = code.to_ascii_uppercase();
    let Some(rule) = ALL_RULES.iter().find(|rule| rule.code == code) else {
        println!("Unknown SKLint rule: {code}");
        return;
    };
    println!(
        "{} {}\n\n{}\n\nFull Markdown help: docs/rules.ru.md",
        rule.code, rule.name, rule.summary
    );
}

fn print_help() {
    println!(
        r#"SKLint {}

Usage:
  sklint check [OPTIONS] [PATHS...]
  sklint check --fix [PATHS...]
  sklint check --format json --stdin-filename path/to/file.py -
  sklint format [--check] [PATHS...]
  sklint format --stdin-filename path/to/file.py -
  sklint rules
  sklint explain SK601

Options:
  --format text|json          Output format, default: text
  --fix                       Apply safe autofixes before reporting diagnostics
  --check                     For format: report files that would change
  --stdin-filename PATH       Project path used when reading source from stdin
  --vscode-strict true|false  VSCode fallback setting; pyproject.toml has priority
  --vscode-select CODES       Comma-separated selectors, e.g. SK601,SK6
  --vscode-ignore CODES       Comma-separated selectors, e.g. SK001

Files:
  Both .py and .pyi files are analyzed.
"#,
        env!("CARGO_PKG_VERSION")
    );
}
