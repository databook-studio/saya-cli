//! The live slash-command popup and the active agent-request state.

use super::super::agent::Stream;
use saya_agent::ApprovalChoice;
use tokio::sync::oneshot;

/// Largest number of text rows the input box grows to before it stops expanding.
pub(crate) const MAX_INPUT_ROWS: usize = 6;

/// A pending tool-approval request awaiting the user's answer.
pub(crate) struct PendingApproval {
    pub(crate) tool: String,
    /// Human-readable detail (e.g. the SQL) shown in the approval dialog.
    pub(crate) detail: Option<String>,
    /// The grammar token a session grant for this call would record; the
    /// modal offers its `[s]` answer only when this is `Some`.
    pub(crate) grant: Option<String>,
    pub(crate) respond: oneshot::Sender<ApprovalChoice>,
}

/// State tied to an active agent request.
#[derive(Default)]
pub(crate) struct RequestState {
    pub(crate) stream: Option<Stream>,
    pub(crate) started: Option<std::time::Instant>,
    pub(crate) activity: Option<String>,
    pub(crate) pending_approval: Option<PendingApproval>,
    /// The last answering call's reported `input_tokens` this turn — the
    /// freshest context-size figure the provider gave, and the numerator for
    /// the footer's context utilisation. Cleared when the request ends so a
    /// later turn never shows a stale figure.
    pub(crate) last_answering_input: Option<u64>,
}
