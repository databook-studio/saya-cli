//! Application state transitions: input, menus, streaming, and clipboard.

mod input_actions;
mod picker;
mod run_panel;
mod search;
mod sql_guard;
mod state;
mod streaming;
mod wide_table;

pub(crate) use sql_guard::SecondSqlDecision;

#[cfg(test)]
pub(crate) mod tests_support;

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
