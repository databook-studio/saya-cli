//! The live slash-command popup and the active agent-request state.

use super::super::agent::Stream;
use saya_agent::{ApprovalChoice, TokenUsage};
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
    /// How many wrapped body rows the panel has scrolled into the fact
    /// text. Per-pending-approval state that dies with it — a fresh
    /// approval starts at the top. Floored at the top as it scrolls and
    /// clamped against the live panel geometry at paint time, so no delta
    /// can scroll the facts out of reach or leave the panel blank.
    pub(crate) scroll: usize,
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
    /// Every provider receipt observed for this in-flight answering run. This
    /// only settles a terminal error; a successful output already owns its
    /// aggregate and must not be counted twice.
    pub(crate) known_answering_usage: Option<TokenUsage>,
}

impl RequestState {
    pub(crate) fn record_answering_usage(&mut self, usage: TokenUsage) {
        let total = self.known_answering_usage.get_or_insert_default();
        total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
        total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
        sum_optional(&mut total.cached_input_tokens, usage.cached_input_tokens);
        sum_optional(
            &mut total.cache_creation_input_tokens,
            usage.cache_creation_input_tokens,
        );
        sum_optional(&mut total.reasoning_tokens, usage.reasoning_tokens);
    }
}

fn sum_optional(total: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or(0).saturating_add(value));
    }
}
