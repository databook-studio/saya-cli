use crate::workspace::EntryKind;
use saya_agent::CancellationToken;
use serde::Serialize;
use std::time::Duration;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    #[error("invalid bounded workspace search arguments")]
    Invalid,
    #[error("workspace search was cancelled")]
    Cancelled,
}

/// The smallest result cap that can hold a truthful outcome: one whose
/// coverage detail was dropped because it did not fit. Below this no bounded
/// answer exists at all, so such limits are invalid rather than answerable —
/// a cap this small is a caller bug, not a search result.
pub const MIN_RESULT_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchStop {
    Deadline,
    Entries,
    Depth,
    ReadBytes,
    Matches,
    ResultBytes,
    Unavailable,
    IgnorePolicyUnavailable,
}

#[derive(Debug, Clone, Copy)]
pub enum SearchKind {
    Glob,
    Grep,
}

/// The `type` filter a glob may state: the two entry kinds a caller can ask
/// for. A glob without one serves both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEntry {
    File,
    Dir,
}
impl SearchEntry {
    pub(super) fn admits(self, kind: EntryKind) -> bool {
        matches!(
            (self, kind),
            (Self::File, EntryKind::File) | (Self::Dir, EntryKind::Dir)
        )
    }
}

/// The most context lines a grep may ask for on each side of a hit.
pub const MAX_CONTEXT: usize = 10;
/// The most extensions one `type` filter may name, and the longest one.
pub const MAX_EXTENSIONS: usize = 8;
pub const MAX_EXTENSION_BYTES: usize = 16;

/// Which entries a grep serves: the matching lines themselves, one path per
/// file holding a match, or one `{path, count}` row per such file. The entry
/// is the unit of `matches`, `matches_found`, `total_matches` and `omitted`
/// alike, so every mode still reports the same coverage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchOutput {
    #[default]
    Content,
    FilesWithMatches,
    Count,
}

/// The grep-only presentation knobs. Every one is bounded — by
/// [`SearchLimits`], by [`MAX_CONTEXT`], or by the extension map's own size —
/// and none raises the match cap, the result cap or the deadline. A glob
/// query must leave the whole struct at its default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrepOptions {
    pub mode: SearchOutput,
    /// Unchanged neighbouring lines served around each hit, on each side.
    pub context: usize,
    /// Match the pattern against the file's whole text rather than each line,
    /// so it can span a line boundary. Not combinable with `context`.
    pub multiline: bool,
    /// The most entries served, never above `SearchLimits::matches`.
    pub head_limit: Option<usize>,
    /// Lowercase extensions without the dot; a file is admitted only when its
    /// own extension is one of them.
    pub extensions: Option<Vec<String>>,
}

/// A query is validated by the owner before filesystem or compilation work.
#[derive(Debug)]
pub struct SearchQuery {
    pub kind: SearchKind,
    pub pattern: String,
    pub path: String,
    pub include: Option<String>,
    pub regex: bool,
    pub case_insensitive: bool,
    /// The glob `type` filter; a grep query must leave it unset.
    pub entry: Option<SearchEntry>,
    /// The grep presentation knobs; a glob query must leave them at default.
    pub grep: GrepOptions,
    /// Trusted application policy only; never accepted from tool arguments.
    pub private_prefixes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SearchLimits {
    pub entries: usize,
    pub read_bytes: usize,
    pub file_bytes: usize,
    pub matches: usize,
    pub line_bytes: usize,
    pub depth: usize,
    pub result_bytes: usize,
    pub timeout: Duration,
}
impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            entries: 100_000,
            read_bytes: 64 * 1024 * 1024,
            file_bytes: 8 * 1024 * 1024,
            matches: 500,
            line_bytes: 2_000,
            depth: 64,
            result_bytes: 48 * 1024,
            timeout: Duration::from_secs(2),
        }
    }
}
impl SearchLimits {
    pub(super) fn validate(&self) -> Result<(), SearchError> {
        let max = Self::default();
        if self.entries > max.entries
            || self.read_bytes > max.read_bytes
            || self.file_bytes > max.file_bytes
            || self.matches > max.matches
            || self.line_bytes > max.line_bytes
            || self.depth > max.depth
            || self.result_bytes > max.result_bytes
            || self.timeout > max.timeout
        {
            return Err(SearchError::Invalid);
        }
        if self.result_bytes < MIN_RESULT_BYTES {
            return Err(SearchError::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum SearchMatch {
    Path(String),
    Line {
        path: String,
        line: usize,
        text: String,
        /// Whether the served text was clipped. Served only when it was: a
        /// line that fits carries no flag, and absence reads as "not clipped".
        #[serde(skip_serializing_if = "is_false")]
        truncated: bool,
        /// Served only as an unchanged neighbour of a hit — it did not match.
        /// Absent on a hit, so a content entry keeps the shape it had before
        /// context existed.
        #[serde(skip_serializing_if = "is_false")]
        context: bool,
    },
    /// One file's own hit count, served by `output_mode: count`.
    Count {
        path: String,
        count: u64,
    },
}
impl SearchMatch {
    pub(super) fn key(&self) -> (&str, usize) {
        match self {
            Self::Path(path) | Self::Count { path, .. } => (path, 0),
            Self::Line { path, line, .. } => (path, *line),
        }
    }
    /// Whether this entry is one the search found by itself. A context line
    /// is served around a hit rather than found, so it never counts against
    /// `matches_found` or the entry cap.
    pub(super) fn found(&self) -> bool {
        !matches!(self, Self::Line { context: true, .. })
    }
}
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug)]
pub struct SearchOutcome {
    pub matches: Vec<SearchMatch>,
    pub matches_found: u64,
    /// Matches the bound or the cap left out of `matches`. A lower bound:
    /// exact when `coverage_complete`, otherwise "at least this many".
    pub omitted: u64,
    pub total_matches: Option<u64>,
    pub truncated: bool,
    pub coverage_complete: bool,
    pub stop_reason: Option<SearchStop>,
    pub entries_visited: usize,
    pub files_scanned: usize,
    pub files_skipped: usize,
    /// Whether the coverage detail — the counts above and the stop reason —
    /// was dropped because it did not fit the result cap. `true` reads as
    /// "this call's coverage is unknown", never as "nothing was covered":
    /// the counts are absent, not zero.
    pub coverage_dropped: bool,
}
impl Default for SearchOutcome {
    fn default() -> Self {
        Self {
            matches: Vec::new(),
            matches_found: 0,
            omitted: 0,
            total_matches: None,
            truncated: false,
            coverage_complete: true,
            stop_reason: None,
            entries_visited: 0,
            files_scanned: 0,
            files_skipped: 0,
            coverage_dropped: false,
        }
    }
}
impl SearchOutcome {
    pub(super) fn incomplete(&mut self, reason: SearchStop) {
        self.coverage_complete = false;
        self.truncated = true;
        self.stop_reason = Some(reason);
    }
    /// Drops the coverage detail — the counts and the stop reason — that no
    /// longer fits the result cap, marking the outcome so a reader sees the
    /// counts are absent rather than zero. The matches are kept: they are the
    /// result, and the cap still bounds them.
    pub(super) fn drop_coverage(&mut self) {
        self.coverage_dropped = true;
        self.coverage_complete = false;
        self.truncated = true;
    }
}
impl Serialize for SearchOutcome {
    /// A coverage-dropped outcome is served without the counts: they are what
    /// did not fit the cap, and emitting them (even zeroed) would both
    /// overflow it and read as a real measurement. Every other outcome keeps
    /// the full field set and order.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        if self.coverage_dropped {
            let mut outcome = serializer.serialize_struct("SearchOutcome", 4)?;
            outcome.serialize_field("matches", &self.matches)?;
            outcome.serialize_field("truncated", &self.truncated)?;
            outcome.serialize_field("coverage_complete", &self.coverage_complete)?;
            outcome.serialize_field("coverage_dropped", &self.coverage_dropped)?;
            return outcome.end();
        }
        let mut outcome = serializer.serialize_struct("SearchOutcome", 11)?;
        outcome.serialize_field("matches", &self.matches)?;
        outcome.serialize_field("matches_found", &self.matches_found)?;
        outcome.serialize_field("omitted", &self.omitted)?;
        outcome.serialize_field("total_matches", &self.total_matches)?;
        outcome.serialize_field("truncated", &self.truncated)?;
        outcome.serialize_field("coverage_complete", &self.coverage_complete)?;
        outcome.serialize_field("stop_reason", &self.stop_reason)?;
        outcome.serialize_field("entries_visited", &self.entries_visited)?;
        outcome.serialize_field("files_scanned", &self.files_scanned)?;
        outcome.serialize_field("files_skipped", &self.files_skipped)?;
        outcome.serialize_field("coverage_dropped", &self.coverage_dropped)?;
        outcome.end()
    }
}

pub(super) struct Control<'a> {
    pub token: &'a CancellationToken,
    pub deadline: Instant,
}
impl Control<'_> {
    pub fn check(&self) -> Result<bool, SearchError> {
        if self.token.is_cancelled() {
            return Err(SearchError::Cancelled);
        }
        Ok(Instant::now() < self.deadline)
    }
}
