//! Full-screen terminal UI for the interactive session.
//!
//! A scrolling transcript region on top, a one-line status bar, a bordered
//! multi-line input box pinned to the bottom, and a slash-command popup that
//! opens the instant the line starts with '/' and filters as you type. Slash
//! commands and `/sql` execute and render into the transcript; live agent
//! streaming arrives in a later milestone. Non-TTY input uses a headless
//! executor, not this module. Rendering lives in `ui`.

pub(crate) mod agent;
mod application;
mod atref;
// pub(crate) so the agent layer's capture hook can reuse the same accounted
// budget as the direct `/sql` path — one source of truth, never a copy.
pub(crate) mod capture;
mod capture_agent;
#[cfg(test)]
mod capture_tests;
mod clipboard;
mod complete;
#[cfg(test)]
#[path = "continuation_acceptance_tests.rs"]
mod continuation_acceptance_tests;
mod dispatch;
mod dispatch_actions;
mod dispatch_contracts;
mod dispatch_investigation;
mod dispatch_runs;
mod exec;
pub(crate) mod export;
mod fuzzy;
mod history;
mod input;
mod keys;
pub(crate) mod loop_tick;
pub(crate) mod replay;
mod replay_task;
mod run_panel;
mod run_panel_apply;
#[cfg(test)]
mod run_panel_snapshot_tests;
#[cfg(test)]
mod run_panel_tests;
mod run_worker;
#[cfg(test)]
mod scope_tests;
pub(crate) mod session;
mod session_save;
mod sql_task;
mod stream_events;
mod table;
mod terminal;
pub(crate) mod transcript;
#[cfg(test)]
mod truncation_tests;
mod trust;
pub(super) mod types;
pub(crate) mod ui;
#[cfg(test)]
mod ui_snapshot_tests;
mod usage_footer;
mod usage_totals;
mod worker_permits;
#[cfg(test)]
mod worker_permits_tests;
pub(crate) mod wrap;

pub(crate) use session::{TrustOutcome, TuiSession, run};
