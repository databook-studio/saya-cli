//! Shared TUI state types used by the event loop, application logic, and renderer.

use super::agent::Stream;
use super::complete::Candidate;
use super::history::History;
use super::input::InputBuffer;
use super::transcript::Transcript;
use crate::config::runtime::RuntimeConfig;
use saya_agent::TokenUsage;
use saya_store::{RedactedSession, SqliteStateStore};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use tokio::sync::oneshot;

/// Session-wide token usage accumulator. Sums every field the usage-accounting slice widened
/// `TokenUsage` with across turns that reported usage. The `Option` fields
/// are tracked with a "was this ever reported?" flag so a cache hit rate over
/// unreported data renders as **unknown**, never 0% — the invariant the usage-accounting slice's
/// `Option` fields exist for (invariant 1: absent is not zero).
///
/// In-memory only: the `SessionState` field carrying this is `#[serde(skip)]`,
/// so it never enters a persisted session file (invariant 2). `/clear` resets
/// it, matching the conversation reset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SessionUsage {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) cache_creation_input_tokens: u64,
    pub(crate) turns: u32,
    /// Whether any turn reported `cached_input_tokens`. When false the cache
    /// hit rate is "unknown" — no provider reported cache data, so a
    /// percentage would be invented. When true, even 0% is an honest
    /// "the cache was cold" (a reported `Some(0)`, not an absent `None`).
    pub(crate) reported_cached: bool,
    pub(crate) reported_cache_creation: bool,
    pub(crate) reported_reasoning: bool,
}

impl SessionUsage {
    /// Folds one turn's usage into the session totals. A turn that reports no
    /// usage (both base counters zero) adds nothing — invariant 4, mirroring
    /// the existing `if usage.input_tokens > 0 || usage.output_tokens > 0`
    /// guard. `AgentOutput.usage` is a bare `TokenUsage` (never `None`), but a
    /// silent provider produces an all-zero one, which this guard skips. The
    /// extraction call's `ChatResponse.usage` is `Option<TokenUsage>`; when Q1
    /// is implemented, `None` maps to "skip" here (absent is not zero).
    pub(crate) fn record(&mut self, usage: &TokenUsage) {
        if usage.input_tokens == 0 && usage.output_tokens == 0 {
            return;
        }
        self.input_tokens += usage.input_tokens;
        self.output_tokens += usage.output_tokens;
        if let Some(cached) = usage.cached_input_tokens {
            self.cached_input_tokens += cached;
            self.reported_cached = true;
        }
        if let Some(created) = usage.cache_creation_input_tokens {
            self.cache_creation_input_tokens += created;
            self.reported_cache_creation = true;
        }
        if let Some(reasoning) = usage.reasoning_tokens {
            self.reasoning_tokens += reasoning;
            self.reported_reasoning = true;
        }
        self.turns += 1;
    }

    /// The session-wide cache hit rate as a percentage string, or "unknown"
    /// when no turn reported cached tokens (invariant 1 / deliverable 5).
    /// The formula is `Σcached / Σinput` — the honest ratio of sums across
    /// all turns, not a mean of per-turn rates. Turns that did not report
    /// cache tokens contribute their input to the denominator but 0 to the
    /// numerator, so the rate is a lower bound, not an invention.
    fn cache_hit_rate(&self) -> String {
        if !self.reported_cached || self.input_tokens == 0 {
            return "unknown".into();
        }
        let rate = (self.cached_input_tokens as f64 / self.input_tokens as f64) * 100.0;
        format!("{rate:.0}%")
    }

    /// Renders the session usage breakdown for `/usage`. Each the usage-accounting slice field shows
    /// its total or `—` when no turn reported it; the hit rate shows the
    /// formula so a reader knows what the number is (Q3, deliverable 4).
    pub(crate) fn render(&self) -> String {
        if self.turns == 0 {
            return "No token usage reported yet this session.".into();
        }
        let dash = "—";
        let opt = |reported: bool, value: u64| -> String {
            if reported {
                value.to_string()
            } else {
                dash.into()
            }
        };
        let rate = self.cache_hit_rate();
        let formula = if self.reported_cached {
            "Σcached / Σinput"
        } else {
            "Σcached / Σinput; no provider reported cache data"
        };
        format!(
            "Session token usage ({turns} turn{plural}):\n\n\
             \x20 Input tokens: {input}\n\
             \x20 Output tokens: {output}\n\
             \x20 Reasoning tokens: {reasoning}\n\
             \x20 Cached input: {cached}\n\
             \x20 Cache creation: {cache_creation}\n\
             \x20 Cache hit rate: {rate} ({formula})",
            turns = self.turns,
            plural = if self.turns == 1 { "" } else { "s" },
            input = self.input_tokens,
            output = self.output_tokens,
            reasoning = opt(self.reported_reasoning, self.reasoning_tokens),
            cached = opt(self.reported_cached, self.cached_input_tokens),
            cache_creation = opt(
                self.reported_cache_creation,
                self.cache_creation_input_tokens
            ),
            rate = rate,
            formula = formula,
        )
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;

/// Largest number of text rows the input box grows to before it stops expanding.
pub(crate) const MAX_INPUT_ROWS: usize = 6;

/// Live slash-command popup state.
pub(crate) struct Menu {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) selected: usize,
}

/// A pending tool-approval request awaiting the user's y/n answer.
pub(crate) struct PendingApproval {
    pub(crate) tool: String,
    /// Human-readable detail (e.g. the SQL) shown in the approval dialog.
    pub(crate) detail: Option<String>,
    pub(crate) respond: oneshot::Sender<bool>,
}

/// A selectable list of saved sessions to resume, filterable as you type.
pub(crate) struct Picker {
    pub(crate) entries: Vec<PickerEntry>,
    pub(crate) selected: usize,
    /// Case-insensitive substring filter over id + label.
    pub(crate) query: String,
}

/// One row in the session picker.
#[derive(Clone)]
pub(crate) struct PickerEntry {
    pub(crate) id: String,
    pub(crate) label: String,
}

/// State tied to an active agent request.
#[derive(Default)]
pub(crate) struct RequestState {
    pub(crate) stream: Option<Stream>,
    pub(crate) started: Option<std::time::Instant>,
    pub(crate) activity: Option<String>,
    pub(crate) pending_approval: Option<PendingApproval>,
}

/// UI overlays and modal interaction state.
/// A Ctrl+R (input history) or Ctrl+F (transcript) search overlay.
pub(crate) struct SearchOverlay {
    pub(crate) kind: SearchKind,
    pub(crate) query: String,
    /// Selected index into the filtered candidate list (history mode).
    pub(crate) selected: usize,
    /// The wrapped-line index the last Enter jumped to (transcript mode), so
    /// the next Enter walks to the *following* match instead of re-landing on
    /// the same one. `None` until the first jump, and reset whenever the query
    /// changes (so an edit restarts the search from the viewport top).
    pub(crate) last_match: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchKind {
    History,
    Transcript,
}

#[derive(Default)]
pub(crate) struct OverlayState {
    pub(crate) menu: Option<Menu>,
    pub(crate) picker_loading: Option<Receiver<Result<Vec<PickerEntry>, String>>>,
    pub(crate) picker: Option<Picker>,
    pub(crate) pending_resume: Option<String>,
    pub(crate) show_help: bool,
    pub(crate) selection_mode: bool,
    pub(crate) search: Option<SearchOverlay>,
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

/// Interactive application state.
pub(crate) struct App {
    pub(crate) input: InputBuffer,
    pub(crate) transcript: Transcript,
    pub(crate) profiles: Vec<String>,
    pub(crate) pending: Option<String>,
    pub(crate) request: RequestState,
    pub(crate) overlays: OverlayState,
    pub(crate) spinner: usize,
    pub(crate) history: History,
    /// Transcript viewport (width, height) captured during the last render,
    /// so key-driven scrolling can clamp to the real size.
    pub(crate) viewport: Cell<(u16, u16)>,
    /// Set after a first Ctrl+C on an empty line; a second one exits.
    pub(crate) ctrl_c_armed: bool,
    /// `@table` / `@table.column` references from the active profiles' cached schema.
    pub(crate) at_refs: Vec<String>,
    /// Text queued for the system clipboard, fulfilled by the run loop via the OS
    /// clipboard tool (pbcopy/wl-copy/xclip/clip) plus an OSC 52 escape for SSH.
    pub(crate) pending_clipboard: Option<String>,
    pub(crate) clipboard_copy: Option<ClipboardCopy>,
    pub(crate) session_save: Option<SessionSave>,
    /// In-flight direct-SQL command (/sql, /export, /chart, /explain) running
    /// off-thread; polled each loop tick so the UI never blocks on a query.
    /// The `Instant` is when the task was dispatched, so the status bar can
    /// show elapsed time alongside the spinner while the query runs.
    pub(crate) sql_task: Option<(
        std::sync::mpsc::Receiver<crate::render::TerminalEvent>,
        super::sql_task::SqlTask,
        std::time::Instant,
    )>,
    pub(crate) pending_session_save: Option<RedactedSession>,
    pub(crate) last_query: Option<LastQuery>,
    pub(crate) runtime: Arc<RuntimeConfig>,
    pub(crate) state_db: SqliteStateStore,
    pub(crate) should_quit: bool,
}
