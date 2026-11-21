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
