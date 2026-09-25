//! The startup trust modal's answers: the TUI's key handling for the one
//! trust decision — trust this folder, name another directory, or continue
//! unbound — opened once after the splash paints. A trust answer binds
//! through `SessionRuntime::bind_trusted`, exactly like an explicit
//! `--workspace`; a refusal or a continued-unbound answer closes the modal
//! with no bind. A bad directory refuses inline (the modal stays, with the
//! error) — never a launch failure, never a silent unbound session.

pub(crate) mod answer;
pub(crate) mod keys;

#[allow(unused_imports)]
pub(crate) use keys::{TrustResolution, trust_key};

// Test-only re-exports so the moved `trust_tests` sibling (via `#[path]`)
// keeps resolving its `use super::*` names.
// Test-only re-exports so the moved `tests` sibling (via `#[path]`)
// keeps resolving its `use super::*` names.
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use super::transcript::BlockKind;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use super::types::{App, TrustPrompt};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use crate::interactive::session_trust;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use ratatui::crossterm::event::{KeyCode, KeyModifiers};

#[cfg(test)]
#[path = "tests.rs"]
mod tests_mod;
