//! In-flight task outcomes and view-local result state.

use std::sync::mpsc::Receiver;

/// The outcome of a `/compact` worker task: either the applied message or a
/// plain failure message (the session is unchanged on every failure path).
/// Carries the compaction call's usage apart from the answering total, folded
/// into the learning total exactly like the post-turn extraction call's.
/// `automatic` names the trigger: a manual `/compact` renders the manual
/// strings, an automatic firing prefixes them so the user knows it was
/// automatic rather than something they typed. The summary rides along so the
/// poll path can apply the working-memory change to the live session — the
/// worker compacted a clone, and the live session must gain exactly what the
/// clone gained, or the transcript would claim a compaction that never
/// happened.
pub(crate) struct CompactOutcome {
    pub(crate) message: String,
    pub(crate) failed: bool,
    pub(crate) usage: Option<saya_agent::TokenUsage>,
    pub(crate) automatic: bool,
    pub(crate) summary: Option<String>,
    pub(crate) compacted_turns: usize,
}

/// A native clipboard helper running in the background alongside an OSC 52 write.
pub(crate) struct ClipboardCopy {
    pub(crate) native_result: Receiver<bool>,
    pub(crate) osc_error: Option<String>,
}

/// A redacted session save running outside the UI event loop.
pub(crate) struct SessionSave {
    pub(crate) result: Receiver<Result<(), String>>,
}

/// The most recent query the app ran (via /sql or an agent tool), so /export
/// (and later /chart) can re-run it. Re-running a SELECT stays read-only.
#[derive(Clone)]
pub(crate) struct LastQuery {
    pub(crate) sql: String,
    pub(crate) connection: Option<String>,
}

/// Presentation state for wide result tables. This is view state: the
/// transcript block text stays the full, untruncated table (what copy and
/// persistence see), and these fields only change how a table is painted.
/// `h_offset` is the first column index shown in the scroll region;
/// `pin_first` holds column 0 in place while the rest scroll; `columns`
/// restricts the view to named columns (`None` shows all).
#[derive(Debug, Clone, Default)]
pub(crate) struct WideTableView {
    pub(crate) h_offset: usize,
    pub(crate) pin_first: bool,
    pub(crate) columns: Option<Vec<String>>,
}
