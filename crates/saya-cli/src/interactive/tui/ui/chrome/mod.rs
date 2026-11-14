//! The persistent frame furniture: context line and status bar.

pub(crate) mod action_line;
mod context_line;
pub(crate) mod status;

// Test-only shim: `status_tests.rs` uses `use super::super::theme::…`, which
// resolves to this module.
#[cfg(test)]
pub(super) use super::theme;

pub(super) use context_line::draw_context_line;
pub(super) use status::draw_status;
