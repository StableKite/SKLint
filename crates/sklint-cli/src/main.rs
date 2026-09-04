use regex::Regex;
use sklint_core::{
    analyze,
    config::{load_pyproject_for_file, validate_selector},
    formatter::format_source,
    pydoclint::{
        validate_pydoclint_config_values_for_path, PydoclintNativeOptions, PydoclintOptions,
    },
    pydoclint_doc::DocStyle,
    rules::ALL_RULES,
    AnalysisInput, Diagnostic, EffectiveConfig, FileInlineConfig, PydoclintCliOverrides,
    VscodeConfig,
};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
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
    native_overrides: PydoclintNativeOverrides,
    group_filenames: bool,
    paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default)]
struct PydoclintNativeOverrides {
    quiet: Option<bool>,
    exclude: Option<String>,
    baseline: Option<PathBuf>,
    generate_baseline: Option<bool>,
    auto_regenerate_baseline: Option<bool>,
    show_filenames_in_every_violation_message: Option<bool>,
}

impl PydoclintNativeOverrides {
    fn apply_to(&self, options: &mut PydoclintNativeOptions) {
        if let Some(value) = self.quiet {
            options.quiet = value;
        }
        if let Some(value) = self.exclude.as_ref() {
            options.exclude = value.clone();
        }
        if let Some(value) = self.baseline.as_ref() {
            options.baseline = Some(value.clone());
        }
        if let Some(value) = self.generate_baseline {
            options.generate_baseline = value;
        }
        if let Some(value) = self.auto_regenerate_baseline {
            options.auto_regenerate_baseline = value;
        }
        if let Some(value) = self.show_filenames_in_every_violation_message {
            options.show_filenames_in_every_violation_message = value;
            options.show_filenames_configured = true;
        }
    }
}

#[derive(Debug, Clone)]
struct FormatArgs {
    check: bool,
    stdin_filename: Option<PathBuf>,
    vscode_config: VscodeConfig,
    paths: Vec<PathBuf>,
    progress_to_stderr: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckFileCandidate {
    path: PathBuf,
    general_sklint: bool,
    pydoclint_native: bool,
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
            print_rules()?;
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
            if args.next().as_deref() == Some("--verbose") {
                print_build_info();
            } else {
                println!("sklint {}", env!("CARGO_PKG_VERSION"));
            }
            Ok(0)
        }
        Some("build-info") => {
            print_build_info();
            Ok(0)
        }
        Some("--help") | Some("-h") | None => {
            print_help();
            Ok(0)
        }
        Some(other) => Err(format!("unknown command `{other}`. Use `sklint --help`.")),
    }
}

fn print_build_info() {
    println!("sklint {}", env!("CARGO_PKG_VERSION"));
    println!("revision: {}", env!("SKLINT_BUILD_REVISION"));
    println!("source-tree-sha256: {}", env!("SKLINT_SOURCE_TREE_SHA256"));
    println!("source-commit: {}", env!("SKLINT_SOURCE_COMMIT"));
    println!("source-dirty: {}", env!("SKLINT_SOURCE_DIRTY"));
    println!("rustc: {}", env!("SKLINT_RUSTC_VERSION"));
    println!("target: {}", env!("SKLINT_BUILD_TARGET"));
}

fn parse_check_args(raw: Vec<String>) -> Result<CheckArgs, String> {
    let mut format = OutputFormat::Text;
    let mut fix = false;
    let mut stdin_filename = None;
    let mut vscode_config = VscodeConfig::default();
    let mut native_overrides = PydoclintNativeOverrides::default();
    let mut group_filenames = false;
    let mut paths = Vec::new();

    let mut idx = 0;
    while idx < raw.len() {
        let (name, inline_value) = split_option(&raw[idx]);
        match name {
            "--format" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--format")?;
                format = parse_output_format(&value)?;
            }
            "--fix" => fix = true,
            "--stdin-filename" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--stdin-filename")?;
                stdin_filename = Some(PathBuf::from(value));
            }
            "--vscode-strict" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--vscode-strict")?;
                parse_vscode_strict_value(&value, &mut vscode_config)?;
            }
            "--vscode-select" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--vscode-select")?;
                vscode_config.select = split_codes(&value);
            }
            "--vscode-ignore" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--vscode-ignore")?;
                vscode_config.ignore = split_codes(&value);
            }
            "--vscode-docstring-style" | "--formatter-docstring-style" => {
                let value = take_required_value(&raw, &mut idx, inline_value, name)?;
                vscode_config.formatter_docstring_style = Some(parse_doc_style(name, &value)?);
            }
            "--config" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--config")?;
                let path = PathBuf::from(value);
                validate_explicit_config_path(&path)?;
                vscode_config.pydoclint_config_path = Some(path);
            }
            "-q" | "--quiet" => native_overrides.quiet = Some(true),
            "--exclude" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--exclude")?;
                native_overrides.exclude = Some(value);
            }
            "--baseline" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--baseline")?;
                native_overrides.baseline = Some(PathBuf::from(value));
            }
            "--generate-baseline" => {
                native_overrides.generate_baseline = Some(take_optional_bool(
                    &raw,
                    &mut idx,
                    inline_value,
                    true,
                    "--generate-baseline",
                )?);
            }
            "-arb" | "--auto-regenerate-baseline" => {
                let value = take_required_value(&raw, &mut idx, inline_value, name)?;
                native_overrides.auto_regenerate_baseline = Some(parse_bool_value(name, &value)?);
            }
            "--no-auto-regenerate-baseline" => {
                native_overrides.auto_regenerate_baseline = Some(false);
            }
            "-sfn" | "--show-filenames-in-every-violation-message" => {
                let value = take_optional_bool(&raw, &mut idx, inline_value, true, name)?;
                native_overrides.show_filenames_in_every_violation_message = Some(value);
            }
            "--group-filenames" => {
                group_filenames = true;
                native_overrides.show_filenames_in_every_violation_message = Some(false);
            }
            "-ths" | "--type-hints-in-signature" => {
                return Err(
                    "`--type-hints-in-signature` was renamed; use `--arg-type-hints-in-signature`"
                        .to_string(),
                );
            }
            "-thd" | "--type-hints-in-docstring" => {
                return Err(
                    "`--type-hints-in-docstring` was renamed; use `--arg-type-hints-in-docstring`"
                        .to_string(),
                );
            }
            "--require-return-section-when-returning-none" => {
                return Err("`--require-return-section-when-returning-none` was renamed; use `--require-return-section-when-returning-nothing`".to_string());
            }
            "-" => paths.push(PathBuf::from("-")),
            other if is_pydoclint_semantic_option(other) => {
                let value = take_required_value(&raw, &mut idx, inline_value, other)?;
                apply_pydoclint_semantic_option(
                    other,
                    &value,
                    &mut vscode_config.pydoclint_overrides,
                )?;
            }
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
        native_overrides,
        group_filenames,
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
        let (name, inline_value) = split_option(&raw[idx]);
        match name {
            "--check" => check = true,
            "--stdin-filename" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--stdin-filename")?;
                stdin_filename = Some(PathBuf::from(value));
            }
            "--vscode-strict" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--vscode-strict")?;
                parse_vscode_strict_value(&value, &mut vscode_config)?;
            }
            "--vscode-select" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--vscode-select")?;
                vscode_config.select = split_codes(&value);
            }
            "--vscode-ignore" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--vscode-ignore")?;
                vscode_config.ignore = split_codes(&value);
            }
            "--vscode-docstring-style" | "--formatter-docstring-style" => {
                let value = take_required_value(&raw, &mut idx, inline_value, name)?;
                vscode_config.formatter_docstring_style = Some(parse_doc_style(name, &value)?);
            }
            "--config" => {
                let value = take_required_value(&raw, &mut idx, inline_value, "--config")?;
                let path = PathBuf::from(value);
                validate_explicit_config_path(&path)?;
                vscode_config.pydoclint_config_path = Some(path);
            }
            "-ths" | "--type-hints-in-signature" => {
                return Err(
                    "`--type-hints-in-signature` was renamed; use `--arg-type-hints-in-signature`"
                        .to_string(),
                );
            }
            "-thd" | "--type-hints-in-docstring" => {
                return Err(
                    "`--type-hints-in-docstring` was renamed; use `--arg-type-hints-in-docstring`"
                        .to_string(),
                );
            }
            "--require-return-section-when-returning-none" => {
                return Err("`--require-return-section-when-returning-none` was renamed; use `--require-return-section-when-returning-nothing`".to_string());
            }
            "-" => paths.push(PathBuf::from("-")),
            other if is_pydoclint_semantic_option(other) => {
                let value = take_required_value(&raw, &mut idx, inline_value, other)?;
                apply_pydoclint_semantic_option(
                    other,
                    &value,
                    &mut vscode_config.pydoclint_overrides,
                )?;
            }
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
        progress_to_stderr: false,
    })
}

fn split_option(arg: &str) -> (&str, Option<&str>) {
    if arg.starts_with('-') {
        if let Some((name, value)) = arg.split_once('=') {
            return (name, Some(value));
        }
    }
    (arg, None)
}

fn take_required_value(
    raw: &[String],
    idx: &mut usize,
    inline: Option<&str>,
    name: &str,
) -> Result<String, String> {
    if let Some(value) = inline {
        return Ok(value.to_string());
    }
    *idx += 1;
    raw.get(*idx)
        .cloned()
        .ok_or_else(|| format!("{name} requires value"))
}

fn take_optional_bool(
    raw: &[String],
    idx: &mut usize,
    inline: Option<&str>,
    default: bool,
    name: &str,
) -> Result<bool, String> {
    if let Some(value) = inline {
        return parse_bool_value(name, value);
    }
    if let Some(next) = raw.get(*idx + 1) {
        if matches!(next.to_ascii_lowercase().as_str(), "true" | "false") {
            *idx += 1;
            return parse_bool_value(name, next);
        }
    }
    Ok(default)
}

fn parse_bool_value(name: &str, value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{name} requires true or false")),
    }
}

fn parse_doc_style(name: &str, value: &str) -> Result<DocStyle, String> {
    DocStyle::parse(value).ok_or_else(|| format!("{name} requires google, numpy or sphinx"))
}

fn validate_explicit_config_path(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!(
            "explicit config file does not exist or is not a file: {}",
            path.display()
        ));
    }
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let has_sklint_config_section = text.lines().any(|raw| {
        let line = raw.split('#').next().unwrap_or("").trim();
        matches!(
            line,
            "[tool.sklint]" | "[tool.pydoclint]" | "[tool.sklint.pydoclint]"
        )
    });
    if !has_sklint_config_section {
        return Err(format!(
            "explicit config file {} does not have a [tool.sklint] section (legacy [tool.pydoclint] aliases are also accepted)",
            path.display()
        ));
    }
    Ok(())
}

fn is_pydoclint_semantic_option(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "--style"
            | "-aths"
            | "--arg-type-hints-in-signature"
            | "-athd"
            | "--arg-type-hints-in-docstring"
            | "-ao"
            | "--check-arg-order"
            | "-scsd"
            | "--skip-checking-short-docstrings"
            | "-scr"
            | "--skip-checking-raises"
            | "-scpf"
            | "--skip-checking-private-functions"
            | "-aid"
            | "--allow-init-docstring"
            | "-crt"
            | "--check-return-types"
            | "-cyt"
            | "--check-yield-types"
            | "-iua"
            | "--ignore-underscore-args"
            | "-ipa"
            | "--ignore-private-args"
            | "-cca"
            | "--check-class-attributes"
            | "-sdpca"
            | "--should-document-private-class-attributes"
            | "-tpmaca"
            | "--treat-property-methods-as-class-attributes"
            | "-oawcv"
            | "--only-attrs-with-classvar-are-treated-as-class-attrs"
            | "-ricvd"
            | "--require-inline-class-var-docs"
            | "-rrs"
            | "--require-return-section-when-returning-nothing"
            | "-rys"
            | "--require-yield-section-when-yielding-nothing"
            | "-sdsa"
            | "--should-document-star-arguments"
            | "-oswdv"
            | "--omit-stars-when-documenting-varargs"
            | "-sdae"
            | "--should-declare-assert-error-if-assert-statement-exists"
            | "--allow-documented-propagated-exceptions"
            | "-csm"
            | "--check-style-mismatch"
            | "-cad"
            | "--check-arg-defaults"
            | "-nmnl"
            | "--native-mode-noqa-location"
    )
}

fn apply_pydoclint_semantic_option(
    name: &str,
    value: &str,
    overrides: &mut PydoclintCliOverrides,
) -> Result<(), String> {
    let name = name.to_ascii_lowercase();
    macro_rules! bool_option {
        ($field:ident) => {{
            overrides.$field = Some(parse_bool_value(&name, value)?);
        }};
    }
    match name.as_str() {
        "--style" => overrides.style = Some(parse_doc_style(&name, value)?),
        "-aths" | "--arg-type-hints-in-signature" => bool_option!(arg_type_hints_in_signature),
        "-athd" | "--arg-type-hints-in-docstring" => bool_option!(arg_type_hints_in_docstring),
        "-ao" | "--check-arg-order" => bool_option!(check_arg_order),
        "-scsd" | "--skip-checking-short-docstrings" => {
            bool_option!(skip_checking_short_docstrings)
        }
        "-scr" | "--skip-checking-raises" => bool_option!(skip_checking_raises),
        "-scpf" | "--skip-checking-private-functions" => {
            bool_option!(skip_checking_private_functions)
        }
        "-aid" | "--allow-init-docstring" => bool_option!(allow_init_docstring),
        "-crt" | "--check-return-types" => bool_option!(check_return_types),
        "-cyt" | "--check-yield-types" => bool_option!(check_yield_types),
        "-iua" | "--ignore-underscore-args" => bool_option!(ignore_underscore_args),
        "-ipa" | "--ignore-private-args" => bool_option!(ignore_private_args),
        "-cca" | "--check-class-attributes" => bool_option!(check_class_attributes),
        "-sdpca" | "--should-document-private-class-attributes" => {
            bool_option!(should_document_private_class_attributes)
        }
        "-tpmaca" | "--treat-property-methods-as-class-attributes" => {
            bool_option!(treat_property_methods_as_class_attributes)
        }
        "-oawcv" | "--only-attrs-with-classvar-are-treated-as-class-attrs" => {
            bool_option!(only_attrs_with_classvar_are_treated_as_class_attrs)
        }
        "-ricvd" | "--require-inline-class-var-docs" => bool_option!(require_inline_class_var_docs),
        "-rrs" | "--require-return-section-when-returning-nothing" => {
            bool_option!(require_return_section_when_returning_nothing)
        }
        "-rys" | "--require-yield-section-when-yielding-nothing" => {
            bool_option!(require_yield_section_when_yielding_nothing)
        }
        "-sdsa" | "--should-document-star-arguments" => {
            bool_option!(should_document_star_arguments)
        }
        "-oswdv" | "--omit-stars-when-documenting-varargs" => {
            bool_option!(omit_stars_when_documenting_varargs)
        }
        "-sdae" | "--should-declare-assert-error-if-assert-statement-exists" => {
            bool_option!(should_declare_assert_error_if_assert_statement_exists)
        }
        "--allow-documented-propagated-exceptions" => {
            bool_option!(allow_documented_propagated_exceptions)
        }
        "-csm" | "--check-style-mismatch" => bool_option!(check_style_mismatch),
        "-cad" | "--check-arg-defaults" => bool_option!(check_arg_defaults),
        "-nmnl" | "--native-mode-noqa-location" => {
            let normalized = value.to_ascii_lowercase();
            if !matches!(normalized.as_str(), "definition" | "docstring") {
                return Err(format!("{name} requires `definition` or `docstring`"));
            }
            overrides.native_mode_noqa_location = Some(normalized);
        }
        _ => return Err(format!("unknown pydoclint option `{name}`")),
    }
    Ok(())
}

fn parse_output_format(value: &str) -> Result<OutputFormat, String> {
    match value {
        "json" => Ok(OutputFormat::Json),
        "text" => Ok(OutputFormat::Text),
        other => Err(format!("unknown format `{other}`")),
    }
}

fn parse_vscode_strict_value(value: &str, vscode_config: &mut VscodeConfig) -> Result<(), String> {
    vscode_config.strict = Some(parse_bool_value("--vscode-strict", value)?);
    Ok(())
}

fn validate_pydoclint_semantics(path: &Path, vscode_config: &VscodeConfig) -> Result<(), String> {
    let project = load_pyproject_for_file(path);
    if !project.errors.is_empty() {
        let config_path = project
            .found_path
            .as_deref()
            .unwrap_or_else(|| Path::new("pyproject.toml"));
        return Err(format!(
            "{}: invalid SKLint configuration: {}",
            config_path.display(),
            project.errors.join("; ")
        ));
    }
    for (kind, selectors) in [
        ("select", &vscode_config.select),
        ("ignore", &vscode_config.ignore),
    ] {
        if let Some(selector) = selectors
            .iter()
            .find(|selector| !validate_selector(selector))
        {
            return Err(format!("invalid --vscode-{kind} selector `{selector}`"));
        }
    }
    let effective = EffectiveConfig::resolve(vscode_config, &project, &FileInlineConfig::default());
    validate_pydoclint_config_values_for_path(
        path,
        effective.pydoclint_inferred_config_context.as_deref(),
        effective.pydoclint_config_path.as_deref(),
        &effective.pydoclint_overrides,
    )
    .map_err(|message| format!("{}: {message}", path.display()))?;
    let options = PydoclintOptions::load_for_path_with_config_context(
        path,
        effective.formatter_docstring_style,
        effective.pydoclint_inferred_config_context.as_deref(),
        effective.pydoclint_config_path.as_deref(),
        &effective.pydoclint_overrides,
    );
    options
        .validate()
        .map_err(|message| format!("{}: {message}", path.display()))
}

fn run_check(args: CheckArgs) -> Result<i32, String> {
    let mut shared_vscode_config = args.vscode_config.clone();
    shared_vscode_config.pydoclint_inferred_config_context = Some(pydoclint_config_context(
        &args.paths,
        args.stdin_filename.as_deref(),
    ));

    if args.fix {
        run_format(FormatArgs {
            check: false,
            stdin_filename: args.stdin_filename.clone(),
            vscode_config: shared_vscode_config.clone(),
            paths: args.paths.clone(),
            // JSON stdout must remain a single valid JSON document.
            progress_to_stderr: args.format == OutputFormat::Json,
        })?;
    }

    let native_options = resolve_native_options(&args)?;
    let exclude = Regex::new(&native_options.exclude).map_err(|err| {
        format!(
            "invalid --exclude regex `{}`: {err}",
            native_options.exclude
        )
    })?;

    let mut diagnostics = Vec::new();
    let mut scanned_files = Vec::new();
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
            let display_key = path_to_posix(&display_path);
            if exclude.is_match(&display_key) {
                continue;
            }
            scanned_files.push(display_key);
            validate_pydoclint_semantics(&display_path, &shared_vscode_config)?;
            let report = analyze(AnalysisInput {
                path: display_path,
                source,
                vscode_config: shared_vscode_config.clone(),
            });
            diagnostics.extend(report.diagnostics);
            continue;
        }

        for candidate in check_files_for_path(path)? {
            let file = candidate.path;
            let file_key = path_to_posix(&file);
            if exclude.is_match(&file_key) {
                continue;
            }
            if candidate.pydoclint_native {
                scanned_files.push(file_key);
                validate_pydoclint_semantics(&file, &shared_vscode_config)?;
            }
            let bytes = fs::read(&file).map_err(|err| format!("{}: {err}", file.display()))?;
            let source = String::from_utf8_lossy(&bytes).into_owned();
            let report = analyze(AnalysisInput {
                path: file,
                source,
                vscode_config: shared_vscode_config.clone(),
            });
            diagnostics.extend(filter_check_diagnostics(
                report.diagnostics,
                candidate.general_sklint,
                candidate.pydoclint_native,
            ));
        }
    }

    // Each per-file AnalysisReport already has product sorting with upstream
    // pydoclint emission order restored for SKD diagnostics. Stable path-only
    // sorting groups multiple inputs without destroying that intra-file order.
    diagnostics.sort_by(|a, b| a.path.cmp(&b.path));

    if native_options.generate_baseline {
        let baseline_path = native_options
            .baseline
            .as_deref()
            .ok_or_else(|| "--generate-baseline requires --baseline PATH".to_string())?;
        let entries = baseline_entries_from_diagnostics(&diagnostics);
        write_baseline(baseline_path, &entries)?;
        if !native_options.quiet {
            eprintln!("The baseline file was successfully generated");
        }
        return Ok(0);
    }

    if let Some(baseline_path) = native_options.baseline.as_deref() {
        if !baseline_path.is_file() {
            return Err(format!(
                "baseline file does not exist: {}. Use --generate-baseline first",
                baseline_path.display()
            ));
        }
        let parsed = parse_baseline(baseline_path)?;
        let evaluated = evaluate_baseline(parsed, diagnostics, &scanned_files);
        diagnostics = evaluated.remaining_diagnostics;
        if evaluated.regeneration_needed {
            if native_options.auto_regenerate_baseline {
                write_baseline(baseline_path, &evaluated.updated_entries)?;
                if !native_options.quiet {
                    eprintln!("Some old violations were fixed; the baseline file was regenerated");
                }
            } else if !native_options.quiet {
                eprintln!(
                    "Some old baseline violations were fixed; regenerate the baseline with --generate-baseline or enable --auto-regenerate-baseline"
                );
            }
        }
    }

    let group_filenames = args.group_filenames
        || (native_options.show_filenames_configured
            && !native_options.show_filenames_in_every_violation_message);
    let output_result = match args.format {
        OutputFormat::Text if group_filenames => print_text_grouped(&diagnostics),
        OutputFormat::Text => print_text(&diagnostics),
        OutputFormat::Json => print_json(&diagnostics),
    };
    ignore_broken_pipe(output_result)?;

    Ok(if diagnostics.is_empty() { 0 } else { 1 })
}

fn pydoclint_config_context(paths: &[PathBuf], stdin_filename: Option<&Path>) -> PathBuf {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut inputs = paths
        .iter()
        .filter(|path| path.as_path() != Path::new("-"))
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                cwd.join(path)
            }
        })
        .collect::<Vec<_>>();

    if inputs.is_empty() {
        if let Some(path) = stdin_filename {
            inputs.push(if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            });
        } else {
            inputs.push(cwd.clone());
        }
    }

    let mut common = inputs.remove(0);
    for input in inputs {
        while !input.starts_with(&common) {
            if !common.pop() {
                return cwd;
            }
        }
    }
    common
}

fn resolve_native_options(args: &CheckArgs) -> Result<PydoclintNativeOptions, String> {
    let context = pydoclint_config_context(&args.paths, args.stdin_filename.as_deref());
    let mut options = PydoclintNativeOptions::load_for_path(&context);
    if let Some(config_path) = args.vscode_config.pydoclint_config_path.as_deref() {
        let text = fs::read_to_string(config_path)
            .map_err(|err| format!("{}: {err}", config_path.display()))?;
        options.apply_toml_text(&text);
    }
    args.native_overrides.apply_to(&mut options);
    if options.generate_baseline && options.baseline.is_none() {
        return Err("--generate-baseline requires --baseline PATH".to_string());
    }
    Ok(options)
}

#[derive(Debug, Clone)]
struct ParsedBaseline {
    order: Vec<String>,
    entries: HashMap<String, Vec<String>>,
}

#[derive(Debug)]
struct BaselineEvaluation {
    remaining_diagnostics: Vec<Diagnostic>,
    updated_entries: Vec<(String, Vec<String>)>,
    regeneration_needed: bool,
}

fn parse_baseline(path: &Path) -> Result<ParsedBaseline, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut order = Vec::new();
    let mut entries: HashMap<String, Vec<String>> = HashMap::new();
    let mut current_file: Option<String> = None;

    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if line == "--------------------" {
            current_file = None;
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        if let Some(file) = current_file.as_ref() {
            // Upstream groups entries by the 20-dash separator and strips
            // whitespace from every line after the filename. Do not require
            // the writer's canonical four-space indent here: tab-indented (or
            // otherwise whitespace-indented) baselines are valid inputs too.
            entries
                .entry(file.clone())
                .or_default()
                .push(line.trim().to_string());
            continue;
        }
        let file = line.trim().to_string();
        if !entries.contains_key(&file) {
            order.push(file.clone());
        }
        // Python dict assignment in upstream replaces an earlier duplicate
        // filename block while preserving the key's original insertion order.
        entries.insert(file.clone(), Vec::new());
        current_file = Some(file);
    }

    Ok(ParsedBaseline { order, entries })
}

fn write_baseline(path: &Path, entries: &[(String, Vec<String>)]) -> Result<(), String> {
    let mut text = String::new();
    for (file, violations) in entries {
        if violations.is_empty() {
            continue;
        }
        text.push_str(file);
        text.push('\n');
        for violation in violations {
            text.push_str("    ");
            text.push_str(violation.trim());
            text.push('\n');
        }
        text.push_str("--------------------\n");
    }
    fs::write(path, text).map_err(|err| format!("{}: {err}", path.display()))
}

fn baseline_entries_from_diagnostics(diagnostics: &[Diagnostic]) -> Vec<(String, Vec<String>)> {
    let mut order = Vec::<String>::new();
    let mut grouped = HashMap::<String, Vec<String>>::new();
    for diagnostic in diagnostics {
        let Some(signature) = baseline_signature(diagnostic) else {
            continue;
        };
        let file = diagnostic.path.replace('\\', "/");
        if !grouped.contains_key(&file) {
            order.push(file.clone());
        }
        grouped.entry(file).or_default().push(signature);
    }
    order
        .into_iter()
        .map(|file| {
            let violations = grouped.remove(&file).unwrap_or_default();
            (file, violations)
        })
        .collect()
}

fn baseline_signature(diagnostic: &Diagnostic) -> Option<String> {
    let code = pydoclint_baseline_code(&diagnostic.code)?;
    Some(format!("{code}: {}", diagnostic.message.trim()))
}

fn pydoclint_baseline_code(code: &str) -> Option<String> {
    let suffix = code.strip_prefix("SKD")?;
    let numeric = suffix.parse::<u16>().ok()?;
    if numeric == 608 {
        return Some("SKD608".to_string());
    }
    if (1..=607).contains(&numeric) {
        return Some(format!("DOC{numeric:03}"));
    }
    None
}

fn evaluate_baseline(
    baseline: ParsedBaseline,
    diagnostics: Vec<Diagnostic>,
    scanned_files: &[String],
) -> BaselineEvaluation {
    let mut unfixed: HashMap<String, Vec<String>> = HashMap::new();
    let mut remaining = Vec::new();

    for diagnostic in diagnostics {
        let file = diagnostic.path.replace('\\', "/");
        let Some(signature) = baseline_signature(&diagnostic) else {
            remaining.push(diagnostic);
            continue;
        };
        let Some(baseline_entries) = baseline.entries.get(&file) else {
            remaining.push(diagnostic);
            continue;
        };
        // Upstream baseline matching is membership-based, not consumptive:
        // every current violation whose signature appears in the baseline is
        // considered baseline-covered. Repeated actual violations therefore
        // remain covered even when the baseline listed the signature once.
        if baseline_entries.iter().any(|entry| entry == &signature) {
            unfixed.entry(file).or_default().push(signature);
        } else {
            remaining.push(diagnostic);
        }
    }

    let scanned: std::collections::HashSet<&str> =
        scanned_files.iter().map(String::as_str).collect();
    let mut regeneration_needed = false;
    let mut updated_entries = Vec::new();
    for file in baseline.order {
        let original = baseline.entries.get(&file).cloned().unwrap_or_default();
        if scanned.contains(file.as_str()) {
            let current = unfixed.remove(&file).unwrap_or_default();
            if current != original {
                regeneration_needed = true;
            }
            if !current.is_empty() {
                updated_entries.push((file, current));
            }
        } else if !original.is_empty() {
            updated_entries.push((file, original));
        }
    }

    BaselineEvaluation {
        remaining_diagnostics: remaining,
        updated_entries,
        regeneration_needed,
    }
}

fn path_to_posix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn run_format(args: FormatArgs) -> Result<i32, String> {
    let mut shared_vscode_config = args.vscode_config.clone();
    if shared_vscode_config
        .pydoclint_inferred_config_context
        .is_none()
    {
        shared_vscode_config.pydoclint_inferred_config_context = Some(pydoclint_config_context(
            &args.paths,
            args.stdin_filename.as_deref(),
        ));
    }

    let mut changed = Vec::new();
    let mut incomplete = Vec::new();
    let mut syntax_blocked = Vec::new();

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
            validate_pydoclint_semantics(&display_path, &shared_vscode_config)?;
            let report = format_source(display_path, source, shared_vscode_config.clone());
            print!("{}", report.source);
            return Ok(if report.remaining_safe == 0 && !report.blocked_by_syntax {
                0
            } else {
                1
            });
        }

        for file in files_for_path(path)? {
            validate_pydoclint_semantics(&file, &shared_vscode_config)?;
            let source =
                fs::read_to_string(&file).map_err(|err| format!("{}: {err}", file.display()))?;
            let report = format_source(file.clone(), source.clone(), shared_vscode_config.clone());
            if report.blocked_by_syntax {
                syntax_blocked.push(file.clone());
            }
            if report.source != source {
                changed.push(file.clone());
                if !args.check {
                    fs::write(&file, report.source)
                        .map_err(|err| format!("{}: {err}", file.display()))?;
                }
            }
            if report.remaining_safe > 0 {
                incomplete.push((file, report.remaining_safe));
            }
        }
    }

    if args.check {
        for file in &changed {
            print_progress(
                &format!("{} needs formatting", file.display()),
                args.progress_to_stderr,
            );
        }
        for (file, count) in &incomplete {
            if !changed.contains(file) {
                print_progress(
                    &format!(
                        "{} has {count} unapplied safe fix{}",
                        file.display(),
                        if *count == 1 { "" } else { "es" }
                    ),
                    args.progress_to_stderr,
                );
            }
        }
        for file in &syntax_blocked {
            print_progress(
                &format!(
                    "{} cannot be formatted safely because full Python AST analysis is unavailable",
                    file.display()
                ),
                args.progress_to_stderr,
            );
        }
        Ok(
            if changed.is_empty() && incomplete.is_empty() && syntax_blocked.is_empty() {
                0
            } else {
                1
            },
        )
    } else {
        for file in &changed {
            print_progress(
                &format!("{} formatted", file.display()),
                args.progress_to_stderr,
            );
        }
        for (file, count) in &incomplete {
            print_progress(
                &format!(
                    "{} still has {count} unapplied safe fix{}",
                    file.display(),
                    if *count == 1 { "" } else { "es" }
                ),
                true,
            );
        }
        for file in &syntax_blocked {
            print_progress(
                &format!(
                    "{} was not modified because full Python AST analysis is unavailable",
                    file.display()
                ),
                true,
            );
        }
        Ok(if incomplete.is_empty() && syntax_blocked.is_empty() {
            0
        } else {
            1
        })
    }
}

fn print_progress(message: &str, to_stderr: bool) {
    if to_stderr {
        eprintln!("{message}");
    } else {
        println!("{message}");
    }
}

fn check_files_for_path(path: &Path) -> Result<Vec<CheckFileCandidate>, String> {
    if path.is_dir() {
        let mut candidates = HashMap::<PathBuf, CheckFileCandidate>::new();

        // General SKLint discovery intentionally supports both .py and .pyi
        // and skips product-noise directories. Keep that behavior unchanged.
        for discovered in collect_python_files(path)? {
            candidates.insert(
                discovered.clone(),
                CheckFileCandidate {
                    path: discovered,
                    general_sklint: true,
                    pydoclint_native: false,
                },
            );
        }

        // pydoclint 0.9.1 uses sorted(Path.rglob("*.py")) for directory
        // inputs and lets its exclude regex decide which matching paths to
        // skip. Use a separate collector so exact native discovery does not
        // remove SKLint's intentional .pyi support or its product-level
        // directory filtering.
        for discovered in collect_pydoclint_python_files(path)? {
            candidates
                .entry(discovered.clone())
                .and_modify(|candidate| candidate.pydoclint_native = true)
                .or_insert(CheckFileCandidate {
                    path: discovered,
                    general_sklint: false,
                    pydoclint_native: true,
                });
        }

        let mut candidates = candidates.into_values().collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.path.cmp(&right.path));
        return Ok(candidates);
    }

    if path.is_file() {
        return Ok(vec![CheckFileCandidate {
            general_sklint: is_python_file(path),
            pydoclint_native: true,
            path: path.to_path_buf(),
        }]);
    }

    // Preserve SKLint's historical missing-Python-path behavior: a missing
    // explicit .py/.pyi path reaches the read step and produces a useful I/O
    // error instead of being silently ignored. Existing non-Python files are
    // handled above as pydoclint-native explicit inputs.
    if is_python_file(path) {
        return Ok(vec![CheckFileCandidate {
            path: path.to_path_buf(),
            general_sklint: true,
            pydoclint_native: true,
        }]);
    }

    Ok(Vec::new())
}

fn filter_check_diagnostics(
    diagnostics: Vec<Diagnostic>,
    general_sklint: bool,
    pydoclint_native: bool,
) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .filter(|diagnostic| {
            let is_pydoclint = diagnostic.code.starts_with("SKD");
            (is_pydoclint && pydoclint_native) || (!is_pydoclint && general_sklint)
        })
        .collect()
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

fn collect_pydoclint_python_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|err| err.to_string())?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|err| err.to_string())?;
            if file_type.is_dir() {
                stack.push(path);
            } else if is_py_file(&path) {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
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

fn is_py_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("py")
}

fn ignore_broken_pipe(result: io::Result<()>) -> Result<(), String> {
    match result {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

fn print_text(diagnostics: &[Diagnostic]) -> io::Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for diag in diagnostics {
        writeln!(
            out,
            "{}:{}:{}: {} {}",
            diag.path, diag.line, diag.column, diag.code, diag.message
        )?;
    }
    Ok(())
}

fn print_text_grouped(diagnostics: &[Diagnostic]) -> io::Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut current_path: Option<&str> = None;
    for diag in diagnostics {
        if current_path != Some(diag.path.as_str()) {
            if current_path.is_some() {
                writeln!(out)?;
            }
            writeln!(out, "{}", diag.path)?;
            current_path = Some(diag.path.as_str());
        }
        writeln!(
            out,
            "    {}:{}: {} {}",
            diag.line, diag.column, diag.code, diag.message
        )?;
    }
    Ok(())
}

fn print_json(diagnostics: &[Diagnostic]) -> io::Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    write!(out, "{{\"diagnostics\":[")?;
    for (idx, diag) in diagnostics.iter().enumerate() {
        if idx > 0 {
            write!(out, ",")?;
        }
        write!(
            out,
            "{{\"code\":\"{}\",\"message\":\"{}\",\"path\":\"{}\",\"line\":{},\"column\":{},\"end_line\":{},\"end_column\":{},\"level\":\"{}\"",
            json_escape(&diag.code),
            json_escape(&diag.message),
            json_escape(&diag.path),
            diag.line,
            diag.column,
            diag.end_line,
            diag.end_column,
            json_escape(&diag.level),
        )?;
        if let Some(line) = diag.suppression_line {
            write!(out, ",\"suppression_line\":{}", line)?;
        }
        if let Some(fix) = &diag.fix {
            write!(
                out,
                ",\"fix\":{{\"safe\":{},\"message\":\"{}\",\"replacement\":\"{}\",\"start_line\":{},\"start_column\":{},\"end_line\":{},\"end_column\":{}}}",
                fix.safe,
                json_escape(&fix.message),
                json_escape(&fix.replacement),
                fix.start_line,
                fix.start_column,
                fix.end_line,
                fix.end_column,
            )?;
        }
        write!(out, "}}")?;
    }
    writeln!(out, "]}}")?;
    Ok(())
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

fn print_rules() -> Result<(), String> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for rule in ALL_RULES {
        if let Err(err) = writeln!(out, "{}\t{:?}\t{}", rule.code, rule.level, rule.summary) {
            if err.kind() == io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(err.to_string());
        }
    }
    Ok(())
}

fn explain(code: &str) {
    let code = code.to_ascii_uppercase();
    let Some(rule) = ALL_RULES.iter().find(|rule| rule.code == code) else {
        println!("Unknown SKLint rule: {code}");
        return;
    };
    println!(
        "{} {}\n\n{}\n\nFull Markdown help: https://github.com/StableKite/SKLint/blob/main/docs/rules.ru.md",
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
  sklint --version [--verbose]
  sklint build-info

Core options:
  --format text|json                 Output format, default: text
  --fix                              Apply safe autofixes before reporting diagnostics
  --check                            For format: report files that would change
  --stdin-filename PATH              Project path used when reading source from stdin
  --vscode-strict true|false         VSCode fallback setting; project config has priority
  --vscode-select CODES              Comma-separated selectors, e.g. SK601,SK6
  --vscode-ignore CODES              Comma-separated selectors, e.g. SK001
  --vscode-docstring-style STYLE     Editor fallback: google|numpy|sphinx
  --formatter-docstring-style STYLE  Formatter fallback: google|numpy|sphinx (default google)

Docstring compatibility config/native options (legacy pydoclint names accepted):
  --config PATH                      Explicit TOML; CLI semantic options win over it
  --exclude REGEX                    Regex search against each POSIX file path
  --baseline PATH                    Ignore matching DOC/SKD baseline violations
  --generate-baseline[=true|false]   Write a baseline and exit
  -arb, --auto-regenerate-baseline BOOL
  --no-auto-regenerate-baseline      Keep a stale baseline until regenerated manually
  -q, --quiet                        Suppress informational baseline messages
  -sfn, --show-filenames-in-every-violation-message[=BOOL]
  --group-filenames                  Group text diagnostics below each filename

Docstring semantic options (legacy pydoclint names accepted):
  --style google|numpy|sphinx
  -aths, --arg-type-hints-in-signature BOOL
  -athd, --arg-type-hints-in-docstring BOOL
  -ao, --check-arg-order BOOL
  -scsd, --skip-checking-short-docstrings BOOL
  -scr, --skip-checking-raises BOOL
  -scpf, --skip-checking-private-functions BOOL
  -aid, --allow-init-docstring BOOL
  -crt, --check-return-types BOOL
  -cyt, --check-yield-types BOOL
  -iua, --ignore-underscore-args BOOL
  -ipa, --ignore-private-args BOOL
  -cca, --check-class-attributes BOOL
  -sdpca, --should-document-private-class-attributes BOOL
  -tpmaca, --treat-property-methods-as-class-attributes BOOL
  -oawcv, --only-attrs-with-ClassVar-are-treated-as-class-attrs BOOL
  -ricvd, --require-inline-class-var-docs BOOL
  -rrs, --require-return-section-when-returning-nothing BOOL
  -rys, --require-yield-section-when-yielding-nothing BOOL
  -sdsa, --should-document-star-arguments BOOL
  -oswdv, --omit-stars-when-documenting-varargs BOOL
  -sdae, --should-declare-assert-error-if-assert-statement-exists BOOL
  --allow-documented-propagated-exceptions BOOL
  -csm, --check-style-mismatch BOOL
  -cad, --check-arg-defaults BOOL
  -nmnl, --native-mode-noqa-location definition|docstring

Precedence for docstring semantics:
  product/editor fallback -> project TOML -> explicit --config -> CLI override

Files:
  Both .py and .pyi files are analyzed.
"#,
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use sklint_core::diagnostic::Span;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn diag(path: &str, code: &str, message: &str) -> Diagnostic {
        Diagnostic::new(code, message, path, Span::new(1, 1, 1, 2), "warning")
    }

    fn temp_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("sklint-cli-{name}-{unique}"))
    }

    #[test]
    fn broken_pipe_on_stdout_is_not_a_cli_error() {
        let error = io::Error::new(io::ErrorKind::BrokenPipe, "consumer closed pipe");
        assert!(ignore_broken_pipe(Err(error)).is_ok());
    }

    #[test]
    fn semantic_cli_accepts_equals_and_short_alias_forms() {
        let args = parse_check_args(vec![
            "--style=sphinx".into(),
            "-ao".into(),
            "false".into(),
            "-nmnl=definition".into(),
            "case.py".into(),
        ])
        .expect("parse");
        assert_eq!(
            args.vscode_config.pydoclint_overrides.style,
            Some(DocStyle::Sphinx)
        );
        assert_eq!(
            args.vscode_config.pydoclint_overrides.check_arg_order,
            Some(false)
        );
        assert_eq!(
            args.vscode_config
                .pydoclint_overrides
                .native_mode_noqa_location
                .as_deref(),
            Some("definition")
        );
    }

    #[test]
    fn deprecated_type_hint_option_is_rejected() {
        let error = parse_check_args(vec!["-ths".into(), "true".into(), "case.py".into()])
            .expect_err("deprecated option must fail");
        assert!(error.contains("arg-type-hints-in-signature"));
    }

    #[test]
    fn inferred_invalid_native_noqa_location_is_rejected_unless_cli_overrides_it() {
        let root = temp_path("invalid-native-noqa-config");
        fs::create_dir_all(&root).expect("mkdir");
        let file = root.join("case.py");
        fs::write(&file, "def f():\n    \"\"\"Doc.\"\"\"\n    pass\n").expect("write source");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.pydoclint]\nnative-mode-noqa-location = 'somewhere'\n",
        )
        .expect("write config");

        let args = parse_check_args(vec![file.display().to_string()]).expect("parse args");
        let mut config = args.vscode_config.clone();
        config.pydoclint_inferred_config_context = Some(root.clone());
        let error = validate_pydoclint_semantics(&file, &config)
            .expect_err("invalid config value must fail");
        assert!(error.contains("native-mode-noqa-location"));
        assert!(error.contains("somewhere"));

        let overriding = parse_check_args(vec![
            "--native-mode-noqa-location=definition".into(),
            file.display().to_string(),
        ])
        .expect("parse override");
        let mut overriding_config = overriding.vscode_config.clone();
        overriding_config.pydoclint_inferred_config_context = Some(root.clone());
        validate_pydoclint_semantics(&file, &overriding_config)
            .expect("valid CLI value overrides invalid inferred config");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn inferred_invalid_style_is_rejected_unless_cli_overrides_it() {
        let root = temp_path("invalid-style-config");
        fs::create_dir_all(&root).expect("mkdir");
        let file = root.join("case.py");
        fs::write(&file, "def f():\n    \"\"\"Doc.\"\"\"\n    pass\n").expect("write source");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.pydoclint]\nstyle = 'rest'\n",
        )
        .expect("write config");

        let args = parse_check_args(vec![file.display().to_string()]).expect("parse args");
        let mut config = args.vscode_config.clone();
        config.pydoclint_inferred_config_context = Some(root.clone());
        let error =
            validate_pydoclint_semantics(&file, &config).expect_err("invalid style must fail");
        assert!(error.contains("--style"));
        assert!(error.contains("rest"));

        let overriding =
            parse_check_args(vec!["--style=google".into(), file.display().to_string()])
                .expect("parse override");
        let mut overriding_config = overriding.vscode_config.clone();
        overriding_config.pydoclint_inferred_config_context = Some(root.clone());
        validate_pydoclint_semantics(&file, &overriding_config)
            .expect("valid CLI style overrides invalid inferred config");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn inferred_invalid_semantic_bool_is_rejected_unless_cli_overrides_it() {
        let root = temp_path("invalid-bool-config");
        fs::create_dir_all(&root).expect("mkdir");
        let file = root.join("case.py");
        fs::write(&file, "def f():\n    \"\"\"Doc.\"\"\"\n    pass\n").expect("write source");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.pydoclint]\ncheck-arg-order = 'maybe'\n",
        )
        .expect("write config");

        let args = parse_check_args(vec![file.display().to_string()]).expect("parse args");
        let mut config = args.vscode_config.clone();
        config.pydoclint_inferred_config_context = Some(root.clone());
        let error =
            validate_pydoclint_semantics(&file, &config).expect_err("invalid bool must fail");
        assert!(error.contains("check-arg-order"));
        assert!(error.contains("maybe"));

        let overriding = parse_check_args(vec![
            "--check-arg-order=false".into(),
            file.display().to_string(),
        ])
        .expect("parse override");
        let mut overriding_config = overriding.vscode_config.clone();
        overriding_config.pydoclint_inferred_config_context = Some(root.clone());
        validate_pydoclint_semantics(&file, &overriding_config)
            .expect("valid CLI bool overrides invalid inferred config");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_sklint_selector_in_project_config_is_rejected() {
        let root = temp_path("unknown-sklint-selector");
        fs::create_dir_all(&root).expect("mkdir");
        let file = root.join("case.py");
        fs::write(&file, "def f(v: int) -> bool:\n    return v == 999\n").expect("source");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.sklint]\nstrict = false\nselect = [\"SK999\"]\n",
        )
        .expect("config");
        let error = validate_pydoclint_semantics(&file, &VscodeConfig::default())
            .expect_err("unknown selector must fail");
        assert!(error.contains("SK999"));
        assert!(error.contains("invalid SKLint configuration"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn non_string_sklint_selector_array_is_rejected() {
        let root = temp_path("wrong-type-sklint-selector");
        fs::create_dir_all(&root).expect("mkdir");
        let file = root.join("case.py");
        fs::write(&file, "x = 1\n").expect("source");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.sklint]\nselect = [123]\n",
        )
        .expect("config");
        let error = validate_pydoclint_semantics(&file, &VscodeConfig::default())
            .expect_err("wrong selector type must fail");
        assert!(error.contains("quoted strings"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn explicit_missing_config_is_rejected() {
        let missing = temp_path("missing.toml");
        let error = parse_check_args(vec![
            "--config".into(),
            missing.display().to_string(),
            "case.py".into(),
        ])
        .expect_err("missing config must fail");
        assert!(error.contains("does not exist"));
    }

    #[test]
    fn explicit_pure_sklint_config_is_accepted_without_legacy_pydoclint_section() {
        let path = temp_path("pure-sklint.toml");
        fs::write(
            &path,
            "[tool.sklint]\nstrict = true\nshould-document-private-class-attributes = true\n",
        )
        .expect("write config");
        let args = parse_check_args(vec![
            "--config".into(),
            path.display().to_string(),
            "case.py".into(),
        ])
        .expect("pure [tool.sklint] config must be accepted");
        assert_eq!(
            args.vscode_config.pydoclint_config_path.as_deref(),
            Some(path.as_path())
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn multipath_native_config_uses_common_parent_context() {
        let root = temp_path("native-common-parent");
        let left = root.join("left");
        let right = root.join("right");
        fs::create_dir_all(&left).expect("create left");
        fs::create_dir_all(&right).expect("create right");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.pydoclint]\nquiet = true\nexclude = 'common'\n",
        )
        .expect("write common config");
        fs::write(
            left.join("pyproject.toml"),
            "[tool.pydoclint]\nquiet = false\nexclude = 'left-only'\n",
        )
        .expect("write left config");
        let left_file = left.join("a.py");
        let right_file = right.join("b.py");
        fs::write(&left_file, "x = 1\n").expect("write left source");
        fs::write(&right_file, "x = 2\n").expect("write right source");

        let args = parse_check_args(vec![
            left_file.display().to_string(),
            right_file.display().to_string(),
        ])
        .expect("parse args");
        assert_eq!(
            pydoclint_config_context(&args.paths, args.stdin_filename.as_deref()),
            root
        );
        let options = resolve_native_options(&args).expect("resolve native options");
        assert!(options.quiet);
        assert_eq!(options.exclude, "common");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_precedence_is_project_then_explicit_then_cli() {
        let root = temp_path("native-precedence");
        fs::create_dir_all(&root).expect("create project");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.pydoclint]\nquiet = false\nexclude = 'project'\nauto-regenerate-baseline = true\n",
        )
        .expect("write project config");
        let explicit = root.join("explicit.toml");
        fs::write(
            &explicit,
            "[tool.pydoclint]\nquiet = true\nexclude = 'explicit'\nauto-regenerate-baseline = false\n",
        )
        .expect("write explicit config");
        let file = root.join("case.py");
        fs::write(&file, "x = 1\n").expect("write source");

        let args = parse_check_args(vec![
            "--config".into(),
            explicit.display().to_string(),
            "--exclude=cli".into(),
            "--auto-regenerate-baseline=true".into(),
            file.display().to_string(),
        ])
        .expect("parse args");
        let options = resolve_native_options(&args).expect("resolve native options");
        assert!(options.quiet, "explicit config must override project TOML");
        assert_eq!(options.exclude, "cli", "CLI must override explicit config");
        assert!(
            options.auto_regenerate_baseline,
            "CLI must override explicit config"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sphinx_arg_defaults_combination_is_rejected_after_config_precedence() {
        let root = temp_path("sphinx-defaults");
        fs::create_dir_all(&root).expect("create project");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.pydoclint]\nstyle = 'sphinx'\ncheck-arg-defaults = true\n",
        )
        .expect("write project config");
        let file = root.join("case.py");
        fs::write(&file, "def f():\n    pass\n").expect("write source");

        let args = parse_check_args(vec![file.display().to_string()]).expect("parse args");
        let error = validate_pydoclint_semantics(&file, &args.vscode_config)
            .expect_err("invalid pydoclint option combination must fail");
        assert!(error.contains("check-arg-defaults"));
        assert!(error.contains("style=sphinx"));

        let overriding = parse_check_args(vec![
            "--check-arg-defaults=false".into(),
            file.display().to_string(),
        ])
        .expect("parse override");
        validate_pydoclint_semantics(&file, &overriding.vscode_config)
            .expect("CLI false override makes the combination valid");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_cli_accepts_baseline_and_regex_options() {
        let args = parse_check_args(vec![
            "--exclude=tests/data|\\.git".into(),
            "--baseline".into(),
            "baseline.txt".into(),
            "--generate-baseline".into(),
            "--no-auto-regenerate-baseline".into(),
            "case.py".into(),
        ])
        .expect("parse");
        assert_eq!(
            args.native_overrides.exclude.as_deref(),
            Some("tests/data|\\.git")
        );
        assert_eq!(
            args.native_overrides.baseline,
            Some(PathBuf::from("baseline.txt"))
        );
        assert_eq!(args.native_overrides.generate_baseline, Some(true));
        assert_eq!(args.native_overrides.auto_regenerate_baseline, Some(false));
    }

    #[test]
    fn baseline_code_maps_upstream_families_but_keeps_extension() {
        assert_eq!(pydoclint_baseline_code("SKD002").as_deref(), Some("DOC002"));
        assert_eq!(pydoclint_baseline_code("SKD607").as_deref(), Some("DOC607"));
        assert_eq!(pydoclint_baseline_code("SKD608").as_deref(), Some("SKD608"));
        assert_eq!(pydoclint_baseline_code("SK401"), None);
    }

    #[test]
    fn baseline_filter_matches_upstream_membership_semantics() {
        let baseline = ParsedBaseline {
            order: vec!["case.py".into()],
            entries: HashMap::from([("case.py".into(), vec!["DOC203: mismatch".into()])]),
        };
        let diagnostics = vec![
            diag("case.py", "SKD203", "mismatch"),
            diag("case.py", "SKD203", "mismatch"),
            diag("case.py", "SKD203", "mismatch"),
            diag("case.py", "SK401", "ordinary SKLint diagnostic"),
        ];
        let result = evaluate_baseline(baseline, diagnostics, &["case.py".into()]);
        assert_eq!(result.remaining_diagnostics.len(), 1);
        assert_eq!(result.remaining_diagnostics[0].code, "SK401");
        assert!(result.regeneration_needed);
        assert_eq!(
            result.updated_entries,
            vec![(
                "case.py".into(),
                vec![
                    "DOC203: mismatch".into(),
                    "DOC203: mismatch".into(),
                    "DOC203: mismatch".into(),
                ],
            )]
        );
    }

    #[test]
    fn baseline_regeneration_drops_fixed_scanned_file_but_keeps_unscanned_files() {
        let baseline = ParsedBaseline {
            order: vec!["fixed.py".into(), "other.py".into()],
            entries: HashMap::from([
                ("fixed.py".into(), vec!["DOC201: old".into()]),
                ("other.py".into(), vec!["DOC501: keep".into()]),
            ]),
        };
        let result = evaluate_baseline(baseline, Vec::new(), &["fixed.py".into()]);
        assert!(result.regeneration_needed);
        assert_eq!(
            result.updated_entries,
            vec![("other.py".into(), vec!["DOC501: keep".into()])]
        );
    }

    #[test]
    fn baseline_parser_replaces_duplicate_filename_blocks_like_upstream() {
        let path = temp_path("baseline-duplicate-file.txt");
        fs::write(
            &path,
            "pkg/case.py\n    DOC201: old\n--------------------\npkg/case.py\n    DOC203: new\n--------------------\n",
        )
        .expect("write duplicate baseline blocks");
        let parsed = parse_baseline(&path).expect("parse duplicate baseline blocks");
        assert_eq!(parsed.order, vec!["pkg/case.py"]);
        assert_eq!(parsed.entries["pkg/case.py"], vec!["DOC203: new"]);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn baseline_parser_accepts_tab_indentation_like_upstream() {
        let path = temp_path("baseline-tabs.txt");
        fs::write(
            &path,
            "pkg/case.py\n\tDOC203: mismatch\n\tDOC501: raised\n--------------------\n",
        )
        .expect("write tab-indented baseline");
        let parsed = parse_baseline(&path).expect("parse tab-indented baseline");
        assert_eq!(parsed.order, vec!["pkg/case.py"]);
        assert_eq!(
            parsed.entries["pkg/case.py"],
            vec!["DOC203: mismatch", "DOC501: raised"]
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn baseline_round_trip_uses_upstream_structure() {
        let path = temp_path("baseline.txt");
        let entries = vec![(
            "pkg/case.py".to_string(),
            vec!["DOC203: mismatch".to_string(), "SKD608: empty".to_string()],
        )];
        write_baseline(&path, &entries).expect("write baseline");
        let text = fs::read_to_string(&path).expect("read baseline");
        assert_eq!(
            text,
            "pkg/case.py\n    DOC203: mismatch\n    SKD608: empty\n--------------------\n"
        );
        let parsed = parse_baseline(&path).expect("parse baseline");
        assert_eq!(parsed.order, vec!["pkg/case.py"]);
        assert_eq!(
            parsed.entries["pkg/case.py"],
            vec!["DOC203: mismatch", "SKD608: empty"]
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn native_directory_discovery_is_py_only_without_dropping_general_pyi() {
        let root = temp_path("native-directory-discovery");
        let hidden = root.join(".hidden");
        fs::create_dir_all(&hidden).expect("create hidden directory");
        let py = root.join("case.py");
        let pyi = root.join("stub.pyi");
        let txt = root.join("notes.txt");
        let hidden_py = hidden.join("hidden.py");
        fs::write(&py, "x = 1\n").expect("write py");
        fs::write(&pyi, "x: int\n").expect("write pyi");
        fs::write(&txt, "x = 2\n").expect("write txt");
        fs::write(&hidden_py, "x = 3\n").expect("write hidden py");

        let candidates = check_files_for_path(&root).expect("discover candidates");
        let by_path = candidates
            .into_iter()
            .map(|candidate| (candidate.path.clone(), candidate))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            by_path.get(&py),
            Some(&CheckFileCandidate {
                path: py.clone(),
                general_sklint: true,
                pydoclint_native: true,
            })
        );
        assert_eq!(
            by_path.get(&pyi),
            Some(&CheckFileCandidate {
                path: pyi.clone(),
                general_sklint: true,
                pydoclint_native: false,
            })
        );
        assert_eq!(
            by_path.get(&hidden_py),
            Some(&CheckFileCandidate {
                path: hidden_py.clone(),
                general_sklint: false,
                pydoclint_native: true,
            })
        );
        assert!(!by_path.contains_key(&txt));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_explicit_file_is_extension_agnostic() {
        let root = temp_path("native-explicit-file");
        fs::create_dir_all(&root).expect("create root");
        let txt = root.join("case.txt");
        let pyi = root.join("case.pyi");
        fs::write(&txt, "def f():\n    pass\n").expect("write txt");
        fs::write(&pyi, "def f() -> None: ...\n").expect("write pyi");

        assert_eq!(
            check_files_for_path(&txt).expect("explicit txt"),
            vec![CheckFileCandidate {
                path: txt.clone(),
                general_sklint: false,
                pydoclint_native: true,
            }]
        );
        assert_eq!(
            check_files_for_path(&pyi).expect("explicit pyi"),
            vec![CheckFileCandidate {
                path: pyi.clone(),
                general_sklint: true,
                pydoclint_native: true,
            }]
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_candidate_filter_keeps_only_eligible_rule_families() {
        let diagnostics = vec![
            diag("case.py", "SKD203", "doc mismatch"),
            diag("case.py", "SK401", "ordinary sklint"),
        ];
        let native_only = filter_check_diagnostics(diagnostics.clone(), false, true);
        assert_eq!(native_only.len(), 1);
        assert_eq!(native_only[0].code, "SKD203");

        let general_only = filter_check_diagnostics(diagnostics.clone(), true, false);
        assert_eq!(general_only.len(), 1);
        assert_eq!(general_only[0].code, "SK401");

        let both = filter_check_diagnostics(diagnostics, true, true);
        assert_eq!(both.len(), 2);
    }

    #[test]
    fn invalid_exclude_regex_is_reported_before_analysis() {
        let args = parse_check_args(vec!["--exclude".into(), "[".into(), "case.py".into()])
            .expect("parse args");
        let native = resolve_native_options(&args).expect("native options");
        assert!(Regex::new(&native.exclude).is_err());
    }
}
