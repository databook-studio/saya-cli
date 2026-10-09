#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashParseError(pub String);

impl fmt::Display for SlashParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SlashParseError {}

/// Which result an `/export` writes: the result the user already inspected
/// (a snapshot of the latest captured `/sql` success, no query) or a fresh
/// read (the last query re-run on its original connection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportMode {
    /// Export the captured result; nothing is executed.
    Snapshot,
    /// Re-run the last query and export the fresh result.
    Refresh,
}

/// The parsed arguments of one `/export`: the mode (`None` is the legacy
/// form, which refreshes), whether an existing destination may be replaced,
/// and the destination path — everything after the flags, whitespace
/// included, so paths with spaces keep working.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRequest {
    pub mode: Option<ExportMode>,
    pub overwrite: bool,
    pub path: String,
}

/// The `--rows` cap for `/report`: at most this many rows of the captured
/// result land in the report's table. The report renderer clamps to the
/// same bound — one source — so a caller that skips the parser cannot
/// exceed it either.
pub(crate) const MAX_REPORT_ROWS: usize = 100;

/// The parsed arguments of one `/report`: the destination path (everything
/// after the flags, verbatim), the row request, and whether an existing
/// destination may be replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRequest {
    pub path: String,
    pub rows: Option<usize>,
    pub overwrite: bool,
}
