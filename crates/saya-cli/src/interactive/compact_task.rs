//! The `/compact` execution: plan, run the bounded summariser, validate, apply
//! or leave untouched.
//!
//! Every surface's `/compact` is the chat session's own compaction
//! (`chat::Compaction::force`), which runs this same engine operation; the old
//! TUI's detached worker (`tui_worker.rs`) went with that surface in X1.

// The manual trigger's spelling is `run_with_trigger(…, false, …)`; `/compact`
// on the plain REPL is the chat session's own compaction, which runs the same
// engine operation.
pub(crate) use crate::interactive::session::continuation::compaction::engine::run_with_trigger;

#[cfg(test)]
#[path = "compact_task_metrics_tests.rs"]
mod compact_task_metrics_tests;
