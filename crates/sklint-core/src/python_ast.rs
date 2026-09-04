//! Shared Python syntax layer backed by `rustpython-parser`.
//!
//! New semantic rules must use this module instead of reconstructing Python
//! structure from indentation or regular expressions.  The parser yields the
//! same broad AST families as CPython (`Stmt`, `Expr`, `Arguments`, ...), with
//! byte ranges that we map back to SKLint line/column spans.

use rustpython_parser::ast::{self, Constant, Expr, Ranged, Stmt};
use rustpython_parser::{Parse, ParseError};
use std::collections::{hash_map::DefaultHasher, BTreeSet, HashMap};
use std::fs;
use std::hash::{Hash, Hasher};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

pub type Suite = ast::Suite;

#[derive(Debug, Clone)]
pub struct PythonAst {
    pub suite: Suite,
    line_starts: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxOracleStatus {
    Valid,
    Invalid,
    Unavailable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FunctionFacts {
    pub has_return: bool,
    pub has_bare_return: bool,
    pub has_yield: bool,
    pub has_raise: bool,
    pub has_assert: bool,
    pub has_implicit_exception_path: bool,
    /// The function contains at least one call expression.  This does not
    /// prove that a particular exception is propagated, but it lets the
    /// docstring layer support an explicit policy for wrapper-heavy code
    /// without pretending that every call is statically understood.
    pub has_call: bool,
    pub raised_exceptions: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct ExceptContext {
    types: Vec<String>,
    binding: Option<String>,
}

impl PythonAst {
    pub fn parse(source: &str, filename: &str) -> Result<Self, ParseError> {
        let suite = match Suite::parse(source, filename) {
            Ok(suite) => suite,
            Err(original_error) => {
                let Some(compat) = rewrite_modern_python_for_embedded_parser(source) else {
                    return Err(original_error);
                };
                match Suite::parse(&compat, filename) {
                    Ok(suite) => suite,
                    Err(_) => return Err(original_error),
                }
            }
        };
        Ok(Self {
            suite,
            // The compatibility rewrite is byte-for-byte length preserving, so
            // AST byte ranges map back to the original source exactly.
            line_starts: line_starts(source),
        })
    }

    pub fn location_of_offset(&self, byte_offset: usize) -> SourceLocation {
        let line_index = match self.line_starts.binary_search(&byte_offset) {
            Ok(index) => index,
            Err(0) => 0,
            Err(index) => index - 1,
        };
        SourceLocation {
            line: line_index + 1,
            column: byte_offset.saturating_sub(self.line_starts[line_index]) + 1,
        }
    }

    pub fn location_of<T: Ranged>(&self, node: &T) -> SourceLocation {
        self.location_of_offset(text_size_to_usize(node.start()))
    }

    pub fn end_location_of<T: Ranged>(&self, node: &T) -> SourceLocation {
        self.location_of_offset(text_size_to_usize(node.end()))
    }
}

/// Ask an installed isolated CPython whether the source is syntactically valid.
///
/// `Unavailable` is deliberately distinct from `Invalid`: an interpreter that
/// cannot be spawned, times out, or otherwise fails as infrastructure must
/// never turn valid modern Python into a false syntax diagnostic.
pub fn cpython_syntax_status(source: &str) -> SyntaxOracleStatus {
    static CACHE: OnceLock<Mutex<HashMap<u64, SyntaxOracleStatus>>> = OnceLock::new();
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    let interpreter_override = std::env::var("SKLINT_PYTHON").ok();
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    interpreter_override.hash(&mut hasher);
    let key = hasher.finish();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(value) = guard.get(&key) {
            return *value;
        }
    }

    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = std::env::temp_dir().join(format!(
        "sklint-cpython-ast-{}-{key:016x}-{counter}.py",
        std::process::id()
    ));
    if fs::write(&temp_path, source).is_err() {
        return SyntaxOracleStatus::Unavailable;
    }

    // The compatibility oracle must understand the newest grammar SKLint
    // claims to accept.  An older host Python rejecting newer syntax is an
    // infrastructure/version mismatch, not proof that the user's source is
    // invalid. Exit 86 is reserved for that state and maps to `Unavailable`.
    const ORACLE_TOO_OLD_EXIT: i32 = 86;
    const SCRIPT: &str = "import ast,sys,pathlib; sys.exit(86) if sys.version_info < (3,13) else ast.parse(pathlib.Path(sys.argv[1]).read_text(encoding='utf-8'))";
    let path_arg = temp_path.to_string_lossy().into_owned();
    let candidates = if let Some(program) = interpreter_override.as_ref() {
        vec![(
            program.clone(),
            vec!["-I", "-S", "-c", SCRIPT, path_arg.as_str()],
        )]
    } else {
        vec![
            (
                "python3".to_string(),
                vec!["-I", "-S", "-c", SCRIPT, path_arg.as_str()],
            ),
            (
                "python".to_string(),
                vec!["-I", "-S", "-c", SCRIPT, path_arg.as_str()],
            ),
            (
                "py".to_string(),
                vec!["-3", "-I", "-S", "-c", SCRIPT, path_arg.as_str()],
            ),
        ]
    };

    let mut saw_completed_rejection = false;
    let mut result = SyntaxOracleStatus::Unavailable;
    for (program, args) in candidates {
        let Ok(mut child) = Command::new(&program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };

        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let Some(status) = status else {
            continue;
        };
        if status.success() {
            result = SyntaxOracleStatus::Valid;
            break;
        }
        if status.code() == Some(ORACLE_TOO_OLD_EXIT) {
            continue;
        }
        saw_completed_rejection = true;
    }

    if result != SyntaxOracleStatus::Valid {
        result = if saw_completed_rejection {
            SyntaxOracleStatus::Invalid
        } else {
            SyntaxOracleStatus::Unavailable
        };
    }

    let _ = fs::remove_file(&temp_path);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, result);
    }
    result
}

pub fn cpython_accepts_source(source: &str) -> bool {
    cpython_syntax_status(source) == SyntaxOracleStatus::Valid
}

/// Apply length-preserving compatibility rewrites for modern Python grammar
/// accepted by target CPython but not yet understood by RustPython 0.4.
fn rewrite_modern_python_for_embedded_parser(source: &str) -> Option<String> {
    let pep701 = rewrite_pep701_same_quote_fstrings(source).unwrap_or_else(|| source.to_string());
    let pep695 = rewrite_pep695_class_type_params(&pep701).unwrap_or_else(|| pep701.clone());
    let pep696 =
        rewrite_pep695_function_and_type_alias_params(&pep695).unwrap_or_else(|| pep695.clone());
    (pep696 != source).then_some(pep696)
}

/// PEP 695 class type parameters (`class Base[T]:`) are not needed for the
/// structural class model itself. Blank only the bracketed declaration while
/// preserving byte offsets; parameterized uses such as `Base[int]` remain AST
/// subscripts and are resolved to their origin class separately.
fn rewrite_pep695_class_type_params(source: &str) -> Option<String> {
    let mut bytes = source.as_bytes().to_vec();
    let original = bytes.clone();
    let mut line_start = 0usize;
    while line_start < bytes.len() {
        let line_end = bytes[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| line_start + offset)
            .unwrap_or(bytes.len());
        let line = &bytes[line_start..line_end];
        let indent = line
            .iter()
            .take_while(|byte| **byte == b' ' || **byte == b'\t')
            .count();
        if line.get(indent..indent + 6) == Some(b"class ") {
            let mut cursor = line_start + indent + 6;
            while cursor < line_end
                && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_')
            {
                cursor += 1;
            }
            if cursor < line_end && bytes[cursor] == b'[' {
                let mut depth = 0usize;
                let mut end = None;
                for (index, byte) in bytes.iter().enumerate().take(line_end).skip(cursor) {
                    match *byte {
                        b'[' => depth += 1,
                        b']' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                end = Some(index);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(end) = end {
                    for byte in &mut bytes[cursor..=end] {
                        *byte = b' ';
                    }
                }
            }
        }
        line_start = if line_end < bytes.len() {
            line_end + 1
        } else {
            bytes.len()
        };
    }
    (bytes != original).then(|| String::from_utf8(bytes).expect("space-only rewrite stays UTF-8"))
}

/// PEP 695/696 function type parameters and `type` aliases are structural
/// metadata for the rules that currently need the embedded AST. Blank their
/// declaration syntax while preserving every byte offset. For a `type` alias,
/// the leading `type ` keyword is also blanked so RustPython sees an ordinary
/// assignment with the same RHS expression.
fn rewrite_pep695_function_and_type_alias_params(source: &str) -> Option<String> {
    let mut bytes = source.as_bytes().to_vec();
    let original = bytes.clone();
    let mut line_start = 0usize;

    while line_start < bytes.len() {
        let line_end = bytes[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| line_start + offset)
            .unwrap_or(bytes.len());
        let line = &bytes[line_start..line_end];
        let indent = line
            .iter()
            .take_while(|byte| **byte == b' ' || **byte == b'\t')
            .count();
        let start = line_start + indent;

        let (mut cursor, type_alias) = if bytes.get(start..start + 4) == Some(b"def ") {
            (start + 4, false)
        } else if bytes.get(start..start + 10) == Some(b"async def ") {
            (start + 10, false)
        } else if bytes.get(start..start + 5) == Some(b"type ") {
            (start + 5, true)
        } else {
            line_start = if line_end < bytes.len() {
                line_end + 1
            } else {
                bytes.len()
            };
            continue;
        };

        while cursor < line_end && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_')
        {
            cursor += 1;
        }
        if cursor < line_end && bytes[cursor] == b'[' {
            let mut depth = 0usize;
            let mut end = None;
            for (index, byte) in bytes.iter().enumerate().take(line_end).skip(cursor) {
                match *byte {
                    b'[' => depth += 1,
                    b']' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            end = Some(index);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(end) = end {
                for byte in &mut bytes[cursor..=end] {
                    *byte = b' ';
                }
                if type_alias {
                    // A module-level `type ` keyword cannot simply become
                    // spaces because that would create unexpected indentation.
                    // Move the alias name into the keyword column and blank
                    // the remainder of the original LHS; the RHS keeps its
                    // original byte offsets.
                    let alias = bytes[start + 5..cursor].to_vec();
                    for byte in &mut bytes[start..=end] {
                        *byte = b' ';
                    }
                    bytes[start..start + alias.len()].copy_from_slice(&alias);
                }
            }
        }

        line_start = if line_end < bytes.len() {
            line_end + 1
        } else {
            bytes.len()
        };
    }

    (bytes != original).then(|| String::from_utf8(bytes).expect("space-only rewrite stays UTF-8"))
}

/// RustPython 0.4 predates the PEP 701 relaxation that permits the quote used
/// by an f-string delimiter to appear in a string literal inside a replacement
/// field. For the common case, change only those inner delimiters to the other
/// quote character. The rewrite preserves byte length and expression meaning.
fn rewrite_pep701_same_quote_fstrings(source: &str) -> Option<String> {
    let mut bytes = source.as_bytes().to_vec();
    let original = bytes.clone();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'#' {
            index = bytes[index..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|offset| index + offset + 1)
                .unwrap_or(bytes.len());
            continue;
        }

        let Some((quote_index, quote, is_fstring, triple)) = string_token_start(&bytes, index)
        else {
            index += 1;
            continue;
        };
        if is_fstring && !triple {
            index = rewrite_fstring_token(&mut bytes, index, quote_index, quote, None);
        } else {
            let delimiter_len = if triple { 3 } else { 1 };
            index = skip_string_token(&bytes, quote_index + delimiter_len, quote, triple);
        }
    }

    (bytes != original).then(|| String::from_utf8(bytes).expect("quote-only rewrite stays UTF-8"))
}

fn rewrite_fstring_token(
    bytes: &mut [u8],
    _prefix_start: usize,
    quote_index: usize,
    original_quote: u8,
    parent_quote: Option<u8>,
) -> usize {
    let chosen_quote = if parent_quote == Some(original_quote) {
        if original_quote == b'"' {
            b'\''
        } else {
            b'"'
        }
    } else {
        original_quote
    };
    if chosen_quote != original_quote {
        bytes[quote_index] = chosen_quote;
    }

    let mut cursor = quote_index + 1;
    let mut brace_depth = 0usize;
    while cursor < bytes.len() {
        if brace_depth == 0 && bytes[cursor] == original_quote {
            if chosen_quote != original_quote {
                bytes[cursor] = chosen_quote;
            }
            return cursor + 1;
        }
        if bytes[cursor] == b'{' {
            if cursor + 1 < bytes.len() && bytes[cursor + 1] == b'{' && brace_depth == 0 {
                cursor += 2;
                continue;
            }
            brace_depth += 1;
            cursor += 1;
            continue;
        }
        if bytes[cursor] == b'}' && brace_depth > 0 {
            brace_depth -= 1;
            cursor += 1;
            continue;
        }
        if brace_depth > 0 && matches!(bytes[cursor], b'\n' | b'\r') {
            bytes[cursor] = b' ';
            cursor += 1;
            continue;
        }
        if brace_depth > 0 && bytes[cursor] == b'#' {
            while cursor < bytes.len() && !matches!(bytes[cursor], b'\n' | b'\r') {
                bytes[cursor] = b' ';
                cursor += 1;
            }
            continue;
        }

        if brace_depth > 0 {
            if let Some((nested_quote_index, nested_quote, is_fstring, nested_triple)) =
                string_token_start(bytes, cursor)
            {
                if nested_quote_index >= cursor {
                    if is_fstring && !nested_triple {
                        if let Some(end) = neutralize_nested_fstring(
                            bytes,
                            cursor,
                            nested_quote_index,
                            nested_quote,
                            chosen_quote,
                        ) {
                            cursor = end;
                            continue;
                        }
                    }

                    let delimiter_len = if nested_triple { 3 } else { 1 };
                    if nested_triple {
                        cursor = skip_string_token(
                            bytes,
                            nested_quote_index + delimiter_len,
                            nested_quote,
                            true,
                        );
                        continue;
                    }

                    let alternate = if chosen_quote == b'"' { b'\'' } else { b'"' };
                    let mut end = nested_quote_index + 1;
                    let mut escaped = false;
                    while end < bytes.len() {
                        let byte = bytes[end];
                        if escaped {
                            escaped = false;
                            end += 1;
                            continue;
                        }
                        if byte == b'\\' {
                            bytes[end] = b'x';
                            if end + 1 < bytes.len() {
                                bytes[end + 1] = b'x';
                                end += 2;
                            } else {
                                end += 1;
                            }
                            continue;
                        }
                        if matches!(byte, b'\n' | b'\r') {
                            break;
                        }
                        if byte == nested_quote {
                            if nested_quote == chosen_quote
                                && !bytes[nested_quote_index + 1..end].contains(&alternate)
                            {
                                bytes[nested_quote_index] = alternate;
                                bytes[end] = alternate;
                            }
                            end += 1;
                            break;
                        }
                        end += 1;
                    }
                    cursor = end;
                    continue;
                }
            }
        }

        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
        } else {
            cursor += 1;
        }
    }
    bytes.len()
}

fn neutralize_nested_fstring(
    bytes: &mut [u8],
    prefix_start: usize,
    quote_index: usize,
    original_quote: u8,
    parent_quote: u8,
) -> Option<usize> {
    let end = scan_fstring_end(bytes, quote_index, original_quote)?;
    let close_index = end.checked_sub(1)?;
    let chosen = if parent_quote == b'"' { b'\'' } else { b'"' };

    for byte in &mut bytes[prefix_start..quote_index] {
        *byte = b' ';
    }
    bytes[quote_index] = chosen;
    bytes[close_index] = chosen;
    // RustPython's pre-PEP-701 f-string expression lexer can still interpret
    // braces/quotes inside a nested string token as f-string syntax. The
    // nested f-string's *value* is irrelevant to file/class/function structure,
    // so replace its payload with inert bytes while keeping the exact span.
    for byte in &mut bytes[quote_index + 1..close_index] {
        // Newlines are deliberately neutralized too.  The rewrite remains
        // byte-for-byte the same length, so all AST ranges still map to the
        // original source through `line_starts`; only the compatibility
        // parser's internal line accounting changes inside this opaque value.
        *byte = b'x';
    }
    Some(end)
}

fn scan_fstring_end(bytes: &[u8], quote_index: usize, quote: u8) -> Option<usize> {
    let mut cursor = quote_index + 1;
    let mut brace_depth = 0usize;
    while cursor < bytes.len() {
        if brace_depth == 0 && bytes[cursor] == quote {
            return Some(cursor + 1);
        }
        if bytes[cursor] == b'{' {
            if cursor + 1 < bytes.len() && bytes[cursor + 1] == b'{' && brace_depth == 0 {
                cursor += 2;
                continue;
            }
            brace_depth += 1;
            cursor += 1;
            continue;
        }
        if bytes[cursor] == b'}' && brace_depth > 0 {
            brace_depth -= 1;
            cursor += 1;
            continue;
        }
        if brace_depth > 0 {
            if let Some((nested_quote_index, nested_quote, is_fstring, nested_triple)) =
                string_token_start(bytes, cursor)
            {
                if nested_quote_index >= cursor {
                    cursor = if is_fstring && !nested_triple {
                        scan_fstring_end(bytes, nested_quote_index, nested_quote)?
                    } else {
                        skip_string_token(
                            bytes,
                            nested_quote_index + if nested_triple { 3 } else { 1 },
                            nested_quote,
                            nested_triple,
                        )
                    };
                    continue;
                }
            }
        }
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
        } else {
            cursor += 1;
        }
    }
    None
}

fn string_token_start(bytes: &[u8], index: usize) -> Option<(usize, u8, bool, bool)> {
    let mut quote_index = index;
    let mut is_fstring = false;
    if !matches!(bytes.get(index), Some(b'\'') | Some(b'"')) {
        let before_is_ident =
            index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_');
        if before_is_ident
            || !matches!(
                bytes[index],
                b'r' | b'R' | b'f' | b'F' | b'b' | b'B' | b'u' | b'U'
            )
        {
            return None;
        }
        while quote_index < bytes.len()
            && matches!(
                bytes[quote_index],
                b'r' | b'R' | b'f' | b'F' | b'b' | b'B' | b'u' | b'U'
            )
            && quote_index - index < 3
        {
            is_fstring |= matches!(bytes[quote_index], b'f' | b'F');
            quote_index += 1;
        }
        if !matches!(bytes.get(quote_index), Some(b'\'') | Some(b'"')) {
            return None;
        }
    }
    let quote = bytes[quote_index];
    let triple = quote_index + 2 < bytes.len()
        && bytes[quote_index + 1] == quote
        && bytes[quote_index + 2] == quote;
    Some((quote_index, quote, is_fstring, triple))
}

fn skip_string_token(bytes: &[u8], mut index: usize, quote: u8, triple: bool) -> usize {
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }
        if bytes[index] == quote {
            if triple {
                if index + 2 < bytes.len() && bytes[index + 1] == quote && bytes[index + 2] == quote
                {
                    return index + 3;
                }
            } else {
                return index + 1;
            }
        }
        index += 1;
    }
    bytes.len()
}

pub fn source_text<'a, T: Ranged>(source: &'a str, node: &T) -> &'a str {
    let start = text_size_to_usize(node.start()).min(source.len());
    let end = text_size_to_usize(node.end()).min(source.len());
    source.get(start..end).unwrap_or("")
}

pub fn expression_text(source: &str, expr: &Expr) -> String {
    source_text(source, expr).trim().to_string()
}

/// Return a dotted qualified name for simple names/attributes.
///
/// Calls are deliberately unwrapped so decorators such as `@dataclass()` and
/// `@typing.dataclass_transform(...)` resolve to the callable name.
pub fn qualified_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Name(node) => Some(node.id.to_string()),
        Expr::Attribute(node) => {
            let base = qualified_name(&node.value)?;
            Some(format!("{base}.{}", node.attr))
        }
        Expr::Call(node) => qualified_name(&node.func),
        _ => None,
    }
}

pub fn final_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Name(node) => Some(node.id.as_str()),
        Expr::Attribute(node) => Some(node.attr.as_str()),
        Expr::Call(node) => final_name(&node.func),
        _ => None,
    }
}

pub fn subscript_base_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Subscript(node) => qualified_name(&node.value),
        _ => None,
    }
}

pub fn string_constant(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Constant(node) => match &node.value {
            Constant::Str(value) => Some(value.as_str()),
            _ => None,
        },
        _ => None,
    }
}

pub fn suite_docstring(body: &[Stmt]) -> Option<&str> {
    let Stmt::Expr(stmt) = body.first()? else {
        return None;
    };
    string_constant(&stmt.value)
}

pub fn suite_docstring_stmt(body: &[Stmt]) -> Option<&Stmt> {
    let stmt = body.first()?;
    let Stmt::Expr(expr_stmt) = stmt else {
        return None;
    };
    string_constant(&expr_stmt.value)?;
    Some(stmt)
}

/// Collect runtime-control-flow facts from a function body.
///
/// Nested function/class definitions are skipped. This is the crucial semantic
/// distinction that text/indentation heuristics cannot model reliably and is
/// required for pydoclint parity around return/yield/raise checks.
pub fn function_facts(body: &[Stmt]) -> FunctionFacts {
    let mut facts = FunctionFacts::default();
    let mut raised = BTreeSet::new();
    walk_statements(body, &mut facts, &mut raised, None);
    facts.raised_exceptions = raised.into_iter().collect();
    facts
}

fn walk_statements(
    body: &[Stmt],
    facts: &mut FunctionFacts,
    raised: &mut BTreeSet<String>,
    active_except: Option<&ExceptContext>,
) {
    for stmt in body {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::AsyncFunctionDef(_) | Stmt::ClassDef(_) => {
                // A nested scope is not part of the enclosing function's
                // return/yield/raise behavior.
            }
            Stmt::Return(node) => {
                facts.has_return = true;
                if node.value.is_none() {
                    facts.has_bare_return = true;
                }
                if let Some(value) = &node.value {
                    walk_expr(value, facts);
                }
            }
            Stmt::Raise(node) => {
                facts.has_raise = true;
                if let Some(exc) = &node.exc {
                    if let Expr::Name(name) = exc.as_ref() {
                        if let Some(context) = active_except {
                            if context.binding.as_deref() == Some(name.id.as_str()) {
                                raised.extend(context.types.iter().cloned());
                            } else if let Some(name) = raised_exception_name(exc, active_except) {
                                raised.insert(name);
                            }
                        } else if let Some(name) = raised_exception_name(exc, active_except) {
                            raised.insert(name);
                        }
                    } else if let Some(name) = raised_exception_name(exc, active_except) {
                        raised.insert(name);
                    }
                    walk_expr(exc, facts);
                } else if let Some(context) = active_except {
                    raised.extend(context.types.iter().cloned());
                }
                if let Some(cause) = &node.cause {
                    walk_expr(cause, facts);
                }
            }
            Stmt::Assert(node) => {
                facts.has_assert = true;
                walk_expr(&node.test, facts);
                if let Some(message) = &node.msg {
                    walk_expr(message, facts);
                }
            }
            Stmt::For(node) => {
                walk_expr(&node.target, facts);
                walk_expr(&node.iter, facts);
                walk_statements(&node.body, facts, raised, active_except);
                walk_statements(&node.orelse, facts, raised, active_except);
            }
            Stmt::AsyncFor(node) => {
                walk_expr(&node.target, facts);
                walk_expr(&node.iter, facts);
                walk_statements(&node.body, facts, raised, active_except);
                walk_statements(&node.orelse, facts, raised, active_except);
            }
            Stmt::While(node) => {
                walk_expr(&node.test, facts);
                walk_statements(&node.body, facts, raised, active_except);
                walk_statements(&node.orelse, facts, raised, active_except);
            }
            Stmt::If(node) => {
                walk_expr(&node.test, facts);
                walk_statements(&node.body, facts, raised, active_except);
                walk_statements(&node.orelse, facts, raised, active_except);
            }
            Stmt::With(node) => {
                for item in &node.items {
                    walk_expr(&item.context_expr, facts);
                    if let Some(vars) = &item.optional_vars {
                        walk_expr(vars, facts);
                    }
                }
                walk_statements(&node.body, facts, raised, active_except);
            }
            Stmt::AsyncWith(node) => {
                for item in &node.items {
                    walk_expr(&item.context_expr, facts);
                    if let Some(vars) = &item.optional_vars {
                        walk_expr(vars, facts);
                    }
                }
                walk_statements(&node.body, facts, raised, active_except);
            }
            Stmt::Match(node) => {
                walk_expr(&node.subject, facts);
                for case in &node.cases {
                    if let Some(guard) = &case.guard {
                        walk_expr(guard, facts);
                    }
                    walk_statements(&case.body, facts, raised, active_except);
                }
            }
            Stmt::Try(node) => {
                walk_statements(&node.body, facts, raised, active_except);
                for handler in &node.handlers {
                    walk_except_handler(handler, facts, raised);
                }
                walk_statements(&node.orelse, facts, raised, active_except);
                walk_statements(&node.finalbody, facts, raised, active_except);
            }
            Stmt::TryStar(node) => {
                walk_statements(&node.body, facts, raised, active_except);
                for handler in &node.handlers {
                    walk_except_handler(handler, facts, raised);
                }
                walk_statements(&node.orelse, facts, raised, active_except);
                walk_statements(&node.finalbody, facts, raised, active_except);
            }
            Stmt::Expr(node) => walk_expr(&node.value, facts),
            Stmt::Assign(node) => {
                for target in &node.targets {
                    walk_expr(target, facts);
                }
                walk_expr(&node.value, facts);
            }
            Stmt::AnnAssign(node) => {
                walk_expr(&node.target, facts);
                walk_expr(&node.annotation, facts);
                if let Some(value) = &node.value {
                    walk_expr(value, facts);
                }
            }
            Stmt::AugAssign(node) => {
                walk_expr(&node.target, facts);
                walk_expr(&node.value, facts);
            }
            Stmt::Delete(node) => {
                for target in &node.targets {
                    walk_expr(target, facts);
                }
            }
            Stmt::TypeAlias(node) => {
                walk_expr(&node.name, facts);
                walk_expr(&node.value, facts);
            }
            Stmt::Import(_)
            | Stmt::ImportFrom(_)
            | Stmt::Global(_)
            | Stmt::Nonlocal(_)
            | Stmt::Pass(_)
            | Stmt::Break(_)
            | Stmt::Continue(_) => {}
        }
    }
}

fn walk_except_handler(
    handler: &ast::ExceptHandler,
    facts: &mut FunctionFacts,
    raised: &mut BTreeSet<String>,
) {
    let ast::ExceptHandler::ExceptHandler(node) = handler;
    let context = ExceptContext {
        types: node
            .type_
            .as_deref()
            .map(exception_type_names)
            .unwrap_or_default(),
        binding: node.name.as_ref().map(ToString::to_string),
    };
    if let Some(type_expr) = &node.type_ {
        walk_expr(type_expr, facts);
    }
    walk_statements(&node.body, facts, raised, Some(&context));
}

fn exception_type_names(expr: &Expr) -> Vec<String> {
    fn handler_name(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Name(node) => Some(node.id.to_string()),
            Expr::Attribute(_) => qualified_name(expr),
            _ => None,
        }
    }
    match expr {
        Expr::Tuple(node) => node.elts.iter().filter_map(handler_name).collect(),
        _ => handler_name(expr).into_iter().collect(),
    }
}

fn raised_exception_name(expr: &Expr, active_except: Option<&ExceptContext>) -> Option<String> {
    match expr {
        Expr::Call(node) => qualified_name(&node.func),
        Expr::Name(node) => {
            if let Some(context) = active_except {
                if context.binding.as_deref() == Some(node.id.as_str()) {
                    // A named re-raise preserves every exception type from a
                    // tuple handler: `except (A, B) as exc: raise exc`.
                    // The caller inserts a single string, so only collapse the
                    // binding when the handler has one exact type; tuple
                    // handlers are expanded by `raised_exception_names`.
                    if context.types.len() == 1 {
                        return context.types.first().cloned();
                    }
                }
            }
            Some(node.id.to_string())
        }
        Expr::Attribute(_) => qualified_name(expr),
        _ => qualified_name(expr),
    }
}

fn walk_expr(expr: &Expr, facts: &mut FunctionFacts) {
    match expr {
        Expr::Yield(node) => {
            facts.has_yield = true;
            if let Some(value) = &node.value {
                walk_expr(value, facts);
            }
        }
        Expr::YieldFrom(node) => {
            facts.has_yield = true;
            walk_expr(&node.value, facts);
        }
        Expr::Lambda(_) => {
            // Treat lambda as a nested callable scope for control-flow facts.
        }
        Expr::BoolOp(node) => {
            for value in &node.values {
                walk_expr(value, facts);
            }
        }
        Expr::NamedExpr(node) => {
            walk_expr(&node.target, facts);
            walk_expr(&node.value, facts);
        }
        Expr::BinOp(node) => {
            if matches!(
                node.op,
                ast::Operator::Div | ast::Operator::FloorDiv | ast::Operator::Mod
            ) {
                facts.has_implicit_exception_path = true;
            }
            walk_expr(&node.left, facts);
            walk_expr(&node.right, facts);
        }
        Expr::UnaryOp(node) => walk_expr(&node.operand, facts),
        Expr::IfExp(node) => {
            walk_expr(&node.test, facts);
            walk_expr(&node.body, facts);
            walk_expr(&node.orelse, facts);
        }
        Expr::Dict(node) => {
            for key in node.keys.iter().flatten() {
                walk_expr(key, facts);
            }
            for value in &node.values {
                walk_expr(value, facts);
            }
        }
        Expr::Set(node) => {
            for value in &node.elts {
                walk_expr(value, facts);
            }
        }
        Expr::ListComp(node) => {
            walk_expr(&node.elt, facts);
            walk_comprehensions(&node.generators, facts);
        }
        Expr::SetComp(node) => {
            walk_expr(&node.elt, facts);
            walk_comprehensions(&node.generators, facts);
        }
        Expr::DictComp(node) => {
            walk_expr(&node.key, facts);
            walk_expr(&node.value, facts);
            walk_comprehensions(&node.generators, facts);
        }
        Expr::GeneratorExp(node) => {
            walk_expr(&node.elt, facts);
            walk_comprehensions(&node.generators, facts);
        }
        Expr::Await(node) => walk_expr(&node.value, facts),
        Expr::Compare(node) => {
            walk_expr(&node.left, facts);
            for value in &node.comparators {
                walk_expr(value, facts);
            }
        }
        Expr::Call(node) => {
            facts.has_call = true;
            walk_expr(&node.func, facts);
            for arg in &node.args {
                walk_expr(arg, facts);
            }
            for keyword in &node.keywords {
                walk_expr(&keyword.value, facts);
            }
        }
        Expr::FormattedValue(node) => {
            walk_expr(&node.value, facts);
            if let Some(spec) = &node.format_spec {
                walk_expr(spec, facts);
            }
        }
        Expr::JoinedStr(node) => {
            for value in &node.values {
                walk_expr(value, facts);
            }
        }
        Expr::Attribute(node) => walk_expr(&node.value, facts),
        Expr::Subscript(node) => {
            facts.has_implicit_exception_path = true;
            walk_expr(&node.value, facts);
            walk_expr(&node.slice, facts);
        }
        Expr::Starred(node) => walk_expr(&node.value, facts),
        Expr::List(node) => {
            for value in &node.elts {
                walk_expr(value, facts);
            }
        }
        Expr::Tuple(node) => {
            for value in &node.elts {
                walk_expr(value, facts);
            }
        }
        Expr::Slice(node) => {
            if let Some(value) = &node.lower {
                walk_expr(value, facts);
            }
            if let Some(value) = &node.upper {
                walk_expr(value, facts);
            }
            if let Some(value) = &node.step {
                walk_expr(value, facts);
            }
        }
        Expr::Constant(_) | Expr::Name(_) => {}
    }
}

fn walk_comprehensions(items: &[ast::Comprehension], facts: &mut FunctionFacts) {
    for item in items {
        walk_expr(&item.target, facts);
        walk_expr(&item.iter, facts);
        for condition in &item.ifs {
            walk_expr(condition, facts);
        }
    }
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

fn text_size_to_usize(size: ast::text_size::TextSize) -> usize {
    u32::from(size) as usize
}

#[cfg(test)]
mod tests {
    #[test]
    fn pep701_same_quote_rewrite_preserves_ast_coverage() {
        let source = "def f(descriptor):\n    return f\"{descriptor[\"width\"]}\"\n";
        let ast = PythonAst::parse(source, "example.py").expect("PEP 701 compatibility parse");
        assert_eq!(ast.suite.len(), 1);
    }

    #[test]
    fn pep701_multiline_replacement_gets_full_embedded_ast() {
        let source = r#"def f(items: list[str]) -> str:
    return f"{"\n".join(
        item
        for item in items
    )}"
"#;
        let ast =
            PythonAst::parse(source, "example.py").expect("multiline PEP 701 compatibility parse");
        assert_eq!(ast.suite.len(), 1);
    }

    #[test]
    fn pep701_triple_nested_fstring_gets_full_embedded_ast() {
        let source = r#"def render(value: int) -> str:
    return f"{str(f"middle {f"inner {value}"}")}"
"#;
        let ast = PythonAst::parse(source, "example.py")
            .expect("triple nested PEP 701 compatibility parse");
        assert_eq!(ast.suite.len(), 1);
    }

    #[test]
    fn pep701_nested_conditional_fstring_gets_full_embedded_ast() {
        let source = r#"def render(key, missing, node_style, NodeType):
    return f"{"" if key == missing or node_style == NodeType.SET else (
        f"{"" if node_style == NodeType.NONE else " "}{key}:"
    )}"
"#;
        let ast = PythonAst::parse(source, "example.py")
            .expect("conditional nested PEP 701 compatibility parse");
        assert_eq!(ast.suite.len(), 1);
    }

    #[test]
    fn pep701_multiline_nested_fstring_payload_gets_full_embedded_ast() {
        let source = r#"def render(value: int) -> str:
    return f"{str(f"middle {
        value
    }")}"
"#;
        let ast = PythonAst::parse(source, "example.py")
            .expect("multiline nested PEP 701 compatibility parse");
        assert_eq!(ast.suite.len(), 1);
    }

    #[test]
    fn pep695_class_type_params_are_length_preserving_for_embedded_parser() {
        let source = "class Base[T]:\n    x: T\n\nclass Child(Base[int]):\n    y: int\n";
        let ast =
            PythonAst::parse(source, "example.py").expect("PEP 695 class compatibility parse");
        assert_eq!(ast.suite.len(), 2);
    }

    #[test]
    fn pep696_function_default_type_param_gets_full_embedded_ast() {
        let source = "def f[T = int](x: T) -> T:\n    return x\n";
        let ast =
            PythonAst::parse(source, "example.py").expect("PEP 696 function compatibility parse");
        assert_eq!(ast.suite.len(), 1);
        assert!(matches!(ast.suite[0], Stmt::FunctionDef(_)));
    }

    #[test]
    fn pep696_type_alias_default_param_gets_full_embedded_ast() {
        let source = "type Vec[T = int] = list[T]\n";
        let rewritten =
            rewrite_pep695_function_and_type_alias_params(source).expect("type alias rewrite");
        assert_eq!(rewritten, "Vec               = list[T]\n");
        let ast =
            PythonAst::parse(source, "example.py").expect("PEP 696 type alias compatibility parse");
        assert_eq!(ast.suite.len(), 1);
        assert!(matches!(ast.suite[0], Stmt::Assign(_)));
    }

    #[test]
    fn pep701_rewrite_does_not_touch_ordinary_string_text() {
        let source = "text = 'example f\\\"{x[\\\"y\\\"]}\\\"'\n";
        assert!(rewrite_pep701_same_quote_fstrings(source).is_none());
    }

    use super::*;

    #[test]
    fn parser_understands_multiline_class_and_annotation() {
        let source = r#"
class Child(
    BaseOne,
    BaseTwo,
):
    value: dict[
        str,
        list[int],
    ]
"#;
        let parsed = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::ClassDef(class) = &parsed.suite[0] else {
            panic!("expected class");
        };
        assert_eq!(class.name.as_str(), "Child");
        assert_eq!(class.bases.len(), 2);
        let Stmt::AnnAssign(field) = &class.body[0] else {
            panic!("expected annotated field");
        };
        assert_eq!(
            expression_text(source, &field.annotation),
            "dict[\n        str,\n        list[int],\n    ]"
        );
    }

    #[test]
    fn function_facts_ignore_nested_functions_and_classes() {
        let source = r#"
def outer(flag: bool):
    def nested():
        raise ValueError()
        yield 1
        return 1

    class Nested:
        def method(self):
            raise RuntimeError()

    if flag:
        raise LookupError()
    return
"#;
        let parsed = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::FunctionDef(function) = &parsed.suite[0] else {
            panic!("expected function");
        };
        let facts = function_facts(&function.body);
        assert!(facts.has_return);
        assert!(facts.has_bare_return);
        assert!(!facts.has_yield);
        assert!(facts.has_raise);
        assert_eq!(facts.raised_exceptions, vec!["LookupError"]);
    }

    #[test]
    fn decorator_qualified_name_unwraps_calls() {
        let source = "@typing.dataclass_transform()\ndef model(cls):\n    return cls\n";
        let parsed = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::FunctionDef(function) = &parsed.suite[0] else {
            panic!("expected function");
        };
        assert_eq!(
            qualified_name(&function.decorator_list[0]).as_deref(),
            Some("typing.dataclass_transform")
        );
    }

    #[test]
    fn named_reraise_uses_bound_exception_types() {
        let source = r#"
def f():
    try:
        work()
    except (ValueError, TypeError) as exc:
        raise exc
"#;
        let parsed = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::FunctionDef(function) = &parsed.suite[0] else {
            panic!("expected function");
        };
        let facts = function_facts(&function.body);
        assert_eq!(facts.raised_exceptions, vec!["TypeError", "ValueError"]);
    }

    #[test]
    fn unrelated_raise_inside_except_is_not_rewritten_to_handler_type() {
        let source = r#"
def f():
    try:
        work()
    except ValueError as exc:
        raise RuntimeError()
"#;
        let parsed = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::FunctionDef(function) = &parsed.suite[0] else {
            panic!("expected function");
        };
        let facts = function_facts(&function.body);
        assert_eq!(facts.raised_exceptions, vec!["RuntimeError"]);
    }

    #[test]
    fn function_facts_record_implicit_exception_paths() {
        let ast = PythonAst::parse(
            "def f(value, items):\n    return 1 / value + items[0]\n",
            "example.py",
        )
        .unwrap();
        let Stmt::FunctionDef(function) = &ast.suite[0] else {
            panic!("function");
        };
        let facts = function_facts(&function.body);
        assert!(facts.has_implicit_exception_path);
    }

    #[test]
    fn bare_reraise_uses_all_handler_types() {
        let source = r#"
def f():
    try:
        work()
    except (LookupError, OSError):
        raise
"#;
        let parsed = PythonAst::parse(source, "example.py").expect("valid Python");
        let Stmt::FunctionDef(function) = &parsed.suite[0] else {
            panic!("expected function");
        };
        let facts = function_facts(&function.body);
        assert_eq!(facts.raised_exceptions, vec!["LookupError", "OSError"]);
    }
}
