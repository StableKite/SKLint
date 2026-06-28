#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub message: String,
    pub replacement: String,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl Span {
    pub const fn new(line: usize, column: usize, end_line: usize, end_column: usize) -> Self {
        Self {
            line,
            column,
            end_line,
            end_column,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
    pub level: String,
    pub fix: Option<Fix>,
    /// Line used for line-level suppressions. For docstring rules this is the
    /// closing docstring line, matching pydoclint-style suppression placement.
    pub suppression_line: Option<usize>,
}

impl Diagnostic {
    pub fn new(
        code: &str,
        message: impl Into<String>,
        path: impl Into<String>,
        span: Span,
        level: impl Into<String>,
    ) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            path: path.into(),
            line: span.line,
            column: span.column,
            end_line: span.end_line,
            end_column: span.end_column,
            level: level.into(),
            fix: None,
            suppression_line: None,
        }
    }

    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.fix = Some(fix);
        self
    }

    pub fn with_suppression_line(mut self, line: usize) -> Self {
        self.suppression_line = Some(line);
        self
    }
}
