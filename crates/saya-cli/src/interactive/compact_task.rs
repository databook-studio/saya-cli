//! The `/compact` execution shared by both surfaces: plan, run the bounded
//! summariser, validate, apply or leave untouched.
//!
//! Headless calls [`run`] directly (it owns a `block_on` seam); the TUI
//! drives [`start`] + [`poll`] so the event loop never blocks on the model.

pub(crate) use crate::interactive::session::continuation::compaction::engine::{
    run, run_with_trigger,
};
mod tui_worker;
pub(crate) use tui_worker::{poll, start, start_automatic};

#[cfg(test)]
#[path = "compact_task_metrics_tests.rs"]
mod compact_task_metrics_tests;
