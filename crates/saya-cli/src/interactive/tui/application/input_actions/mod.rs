//! Input editing, autocomplete popup, history recall, and clipboard queueing.

mod clipboard;
mod menu;
mod submit;

// Test-only re-export so the `mod tests` sibling (moved byte-identical via
// `#[path]`) keeps resolving its `use super::*` names.
#[cfg(test)]
pub(crate) use crate::interactive::tui::transcript::BlockKind;

#[cfg(test)]
#[path = "input_actions_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "queue_tests.rs"]
mod queue_tests;

#[cfg(test)]
#[path = "revise_tests.rs"]
mod revise_tests;
