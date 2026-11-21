//! The `/compact` execution shared by both surfaces: plan, run the bounded
//! summariser, validate, apply or leave untouched.
//!
//! The TUI drives [`start`] + [`poll`] so the event loop never blocks on the
//! model, and fires the automatic trigger beside them ([`start_automatic`]).
//! The plain REPL's `/compact` is the chat session's own compaction
//! (`chat::Compaction::force`), which runs this same engine operation.

// The manual trigger's spelling is `run_with_trigger(…, false, …)`; `/compact`
// on the plain REPL is the chat session's own compaction, which runs the same
// engine operation.
pub(crate) use crate::interactive::session::continuation::compaction::engine::run_with_trigger;
mod tui_worker;
pub(crate) use tui_worker::{poll, start, start_automatic};

#[cfg(test)]
#[path = "compact_task_metrics_tests.rs"]
mod compact_task_metrics_tests;
