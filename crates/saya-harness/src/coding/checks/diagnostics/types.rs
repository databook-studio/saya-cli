use std::fmt;

/// A compiler-reported diagnostic level supported by the selected grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Note,
    Help,
    FailureNote,
    InternalCompilerError,
}

/// A selected primary span, normalized to a safe candidate-relative path.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiagnosticLocation {
    path: String,
    line_start: u32,
    line_end: u32,
    column_start: u32,
    column_end: u32,
}

impl DiagnosticLocation {
    pub(super) fn new(
        path: String,
        line_start: u32,
        line_end: u32,
        column_start: u32,
        column_end: u32,
    ) -> Self {
        Self {
            path,
            line_start,
            line_end,
            column_start,
            column_end,
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn line_start(&self) -> u32 {
        self.line_start
    }

    pub fn line_end(&self) -> u32 {
        self.line_end
    }

    pub fn column_start(&self) -> u32 {
        self.column_start
    }

    pub fn column_end(&self) -> u32 {
        self.column_end
    }
}

impl fmt::Debug for DiagnosticLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiagnosticLocation")
            .field("path_bytes", &self.path.len())
            .field("line_start", &self.line_start)
            .field("line_end", &self.line_end)
            .field("column_start", &self.column_start)
            .field("column_end", &self.column_end)
            .finish()
    }
}

/// One structured compiler diagnostic. Message and path are available to the
/// trusted parser consumer, but omitted from Debug output.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Diagnostic {
    severity: DiagnosticSeverity,
    code: Option<String>,
    message: String,
    locations: Vec<DiagnosticLocation>,
}

impl Diagnostic {
    pub(super) fn new(
        severity: DiagnosticSeverity,
        code: Option<String>,
        message: String,
        locations: Vec<DiagnosticLocation>,
    ) -> Self {
        Self {
            severity,
            code,
            message,
            locations,
        }
    }

    pub fn severity(&self) -> DiagnosticSeverity {
        self.severity
    }

    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn locations(&self) -> &[DiagnosticLocation] {
        &self.locations
    }
}

impl fmt::Debug for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Diagnostic")
            .field("message_bytes", &self.message.len())
            .field("code_present", &self.code.is_some())
            .field("location_count", &self.locations.len())
            .finish()
    }
}

/// Complete parse output for one explicitly selected parser revision.
#[derive(Clone, PartialEq, Eq)]
pub struct DiagnosticSet {
    entries: Vec<Diagnostic>,
}

impl DiagnosticSet {
    pub(super) fn new(entries: Vec<Diagnostic>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &[Diagnostic] {
        &self.entries
    }

    pub fn parser_revision(&self) -> u32 {
        1
    }
}

impl fmt::Debug for DiagnosticSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiagnosticSet")
            .field("diagnostic_count", &self.entries.len())
            .field("parser_revision", &self.parser_revision())
            .finish()
    }
}

/// A parse refusal safe to display: it contains no tool output or path text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DiagnosticError {
    #[error("diagnostic output is not valid UTF-8")]
    InvalidUtf8,
    #[error("diagnostic output exceeds its byte limit")]
    InputTooLarge,
    #[error("diagnostic record at line {line} exceeds its byte limit")]
    RecordTooLarge { line: usize },
    #[error("diagnostic output has an invalid record at line {line}")]
    InvalidRecord { line: usize },
    #[error("diagnostic record at line {line} is malformed or out of bounds")]
    InvalidDiagnostic { line: usize },
    #[error("diagnostic location at line {line} is unsafe or out of bounds")]
    InvalidLocation { line: usize },
    #[error("diagnostic output exceeds its record count limit")]
    TooManyDiagnostics,
    #[error("diagnostic record does not match the selected format")]
    UnsupportedFormat,
}
