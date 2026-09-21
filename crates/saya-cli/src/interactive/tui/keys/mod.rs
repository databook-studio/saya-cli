//! Keyboard input handling for the TUI event loop.

mod approvals;
mod dispatch;

pub(crate) use dispatch::handle_key;

// Test-only bare-key shims so the `*_tests` siblings (moved byte-identical
// via `#[path]`) keep resolving their `use super::*` names: the production
// helpers take the held modifiers, and `NONE` is exactly the bare press
// those tests pin.
#[cfg(test)]
pub(crate) fn approval_answer(code: KeyCode) -> Option<bool> {
    approvals::approval_answer(code, KeyModifiers::NONE)
}

#[cfg(test)]
pub(crate) fn approval_choice(code: KeyCode, grant: Option<&str>) -> Option<ApprovalChoice> {
    approvals::approval_choice(code, KeyModifiers::NONE, grant)
}

#[cfg(test)]
use super::types::App;
#[cfg(test)]
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
#[cfg(test)]
use saya_agent::ApprovalChoice;

#[cfg(test)]
#[path = "approval_modal_tests.rs"]
mod approval_modal_tests;
#[cfg(test)]
#[path = "approval_modifier_tests.rs"]
mod approval_modifier_tests;
#[cfg(test)]
#[path = "esc_run_panel_tests.rs"]
mod esc_run_panel_tests;
#[cfg(test)]
#[path = "esc_sql_task_tests.rs"]
mod esc_sql_task_tests;
#[cfg(test)]
#[path = "new_activity_tests.rs"]
mod new_activity_tests;
