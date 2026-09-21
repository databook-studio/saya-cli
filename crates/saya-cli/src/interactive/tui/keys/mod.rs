//! Keyboard input handling for the TUI event loop.

mod approvals;
mod dispatch;

pub(crate) use dispatch::handle_key;

// Test-only re-exports so the `*_tests` siblings (moved byte-identical via
// `#[path]`) keep resolving their `use super::*` names.
#[cfg(test)]
pub(crate) use approvals::{approval_answer, approval_choice};

#[cfg(test)]
use super::types::App;
#[cfg(test)]
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

#[cfg(test)]
#[path = "approval_modal_tests.rs"]
mod approval_modal_tests;
#[cfg(test)]
#[path = "esc_run_panel_tests.rs"]
mod esc_run_panel_tests;
#[cfg(test)]
#[path = "esc_sql_task_tests.rs"]
mod esc_sql_task_tests;
#[cfg(test)]
#[path = "new_activity_tests.rs"]
mod new_activity_tests;
