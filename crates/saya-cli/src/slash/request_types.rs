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

/// Which file one `/export` writes: rows as a data file, or today's Markdown
/// report. The extension or `--format` selects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Rows as CSV.
    Csv,
    /// Rows as JSON.
    Json,
    /// Today's `/report` bytes: the Markdown document with provenance.
    Markdown,
}

impl ExportFormat {
    /// The format a `--format` word names; `None` for anything else, so an
    /// unknown word is a usage error rather than a silent default.
    pub(crate) fn parse(word: &str) -> Option<Self> {
        match word {
            "csv" => Some(Self::Csv),
            "json" => Some(Self::Json),
            "md" => Some(Self::Markdown),
            _ => None,
        }
    }

    /// The format a destination's extension names; `None` for an extension
    /// that names no format (`/export out.txt` is refused by the writer).
    pub(crate) fn of_path(path: &str) -> Option<Self> {
        let lowered = path.to_lowercase();
        if lowered.ends_with(".csv") {
            Some(Self::Csv)
        } else if lowered.ends_with(".json") {
            Some(Self::Json)
        } else if lowered.ends_with(".md") {
            Some(Self::Markdown)
        } else {
            None
        }
    }

    /// The `--format` word that names this format.
    pub(crate) const fn word(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
            Self::Markdown => "md",
        }
    }
}

/// The parsed arguments of one `/export`: the mode (`None` is the legacy
/// form, which refreshes), whether an existing destination may be replaced,
/// how many rows a Markdown report carries, the format stated with
/// `--format`, and the destination path — everything after the flags,
/// whitespace included, so paths with spaces keep working.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportRequest {
    pub mode: Option<ExportMode>,
    pub overwrite: bool,
    /// `--rows N` — the Markdown report's table bound.
    pub rows: Option<usize>,
    /// `--format` as stated; `None` lets the destination's extension select.
    pub format: Option<ExportFormat>,
    pub path: String,
    /// The deprecated `/report` spelling stated it (one release only).
    pub via_report_alias: bool,
}

impl ExportRequest {
    /// The format this request writes: `--format` when stated, else the
    /// destination's extension. `None` names no format at all.
    pub fn format(&self) -> Option<ExportFormat> {
        self.format.or(ExportFormat::of_path(&self.path))
    }

    /// Whether this request writes today's Markdown report — a `.md`
    /// destination, or `--format md`.
    pub fn is_report(&self) -> bool {
        self.format() == Some(ExportFormat::Markdown)
    }
}

/// The `--rows` cap for `/report`: at most this many rows of the captured
/// result land in the report's table. The report renderer clamps to the
/// same bound — one source — so a caller that skips the parser cannot
/// exceed it either.
pub(crate) const MAX_REPORT_ROWS: usize = 100;

/// The parsed arguments of one `/permissions`, and which spelling stated it.
///
/// `/grants` is this command's one-release alias, so it produces **this**
/// request with [`Self::via_grants_alias`] set: one view, two spellings, and
/// the note rides the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PermissionsRequest {
    /// The deprecated `/grants` spelling stated it (one release only).
    pub via_grants_alias: bool,
}

impl PermissionsRequest {
    /// `/permissions`, the one read-only view.
    pub(crate) const fn permissions() -> Self {
        Self {
            via_grants_alias: false,
        }
    }

    /// `/grants`, the one-release alias of `/permissions`.
    pub(crate) const fn grants() -> Self {
        Self {
            via_grants_alias: true,
        }
    }
}

/// One `/compact` request: whether the compaction rolls into a new session
/// afterwards, and which spelling stated it.
///
/// `/rotate` is this command's one-release alias, so it produces **this**
/// request with [`Self::via_rotate_alias`] set: one operation, two spellings,
/// and the note is the operation's own answer rather than a second command's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompactRequest {
    /// Compact, then continue in a new session that keeps the summary.
    pub new_session: bool,
    /// The deprecated `/rotate` spelling stated it (one release only).
    pub via_rotate_alias: bool,
}

impl CompactRequest {
    /// Bare `/compact`: the compaction of this session, and nothing else.
    pub(crate) const fn compact() -> Self {
        Self {
            new_session: false,
            via_rotate_alias: false,
        }
    }

    /// `/compact --new-session`.
    pub(crate) const fn new_session() -> Self {
        Self {
            new_session: true,
            via_rotate_alias: false,
        }
    }

    /// `/rotate`, the one-release alias of `/compact --new-session`.
    pub(crate) const fn rotate() -> Self {
        Self {
            new_session: true,
            via_rotate_alias: true,
        }
    }
}

/// One `/sessions` request: the bare listing, and which spelling stated it.
///
/// `/history` is this command's one-release alias, so it produces this request
/// with [`Self::via_history_alias`] set: one listing, two spellings, and the
/// note rides the listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionsRequest {
    /// The deprecated `/history` spelling stated it (one release only).
    pub via_history_alias: bool,
}

impl SessionsRequest {
    /// `/history`, the one-release alias of `/sessions`.
    pub(crate) const fn history() -> Self {
        Self {
            via_history_alias: true,
        }
    }
}

/// The parsed `/rewind` tail: bare is the run's checkpoint listing, one id is
/// that checkpoint restored. The id is carried as stated — resolving it
/// against the listing is the operation's job, so an id this run never
/// checkpointed is refused by name rather than read as a different turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewindRequest {
    /// List the run's checkpoints, newest first.
    List,
    /// Restore the checkpoint `id` names.
    Restore(String),
}

/// One `/usage` request: bare shows the session's usage panel, and
/// `--limit <usd>` also states this session's provider-reported cost ceiling.
/// The ceiling is the session's alone — it is never written to config — and a
/// later `/usage --limit` replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsageRequest {
    /// The ceiling the session's reported spend must stay below, in
    /// micro-USD. `None` shows the panel without changing the ceiling.
    pub limit_micros: Option<u64>,
}

impl UsageRequest {
    /// Bare `/usage`: the panel, and no change to the ceiling.
    pub(crate) const fn show() -> Self {
        Self { limit_micros: None }
    }

    /// `/usage --limit <usd>`: the panel, with this session's ceiling stated.
    pub(crate) const fn limit(micros: u64) -> Self {
        Self {
            limit_micros: Some(micros),
        }
    }
}
