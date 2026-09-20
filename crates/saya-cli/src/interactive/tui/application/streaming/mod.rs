//! Agent streaming lifecycle: starting the background agent prompt and
//! draining its channel into the transcript.
//!
//! Thin module surface: `start_agent` lives in `start`, `drain_stream` in
//! `drain`. Sibling test files exercise the drain seam.

mod drain;
mod start;

// Re-exported so the `#[path]` sibling test modules (which resolve `super::`
// to this module) keep seeing the names the inline `streaming::tests` module
// saw: `App`/`SessionState` plus the stream/test-support names.
// `auto_compact_tests.rs` (still owned by `application/`) uses `super::*`
// the same way. `#[allow]` because only the test configuration consumes them.
#[allow(unused_imports)]
pub(crate) use super::super::agent::{Stream, StreamMsg};
#[allow(unused_imports)]
pub(crate) use super::super::transcript::BlockKind;
#[allow(unused_imports)]
pub(crate) use super::super::types::App;
#[cfg(test)]
pub(crate) use super::tests_support::{idle_app, unused_runtime};
#[allow(unused_imports)]
pub(crate) use crate::interactive::session_state::SessionState;
#[allow(unused_imports)]
pub(crate) use saya_agent::{AgentEvent, AgentOutput, CancellationToken, TokenUsage, UsageCall};
#[allow(unused_imports)]
pub(crate) use std::sync::Arc;
#[allow(unused_imports)]
pub(crate) use tokio::sync::mpsc::unbounded_channel;

#[cfg(test)]
#[path = "../auto_compact_tests.rs"]
mod auto_compact_tests;
#[cfg(test)]
#[path = "footer_tests.rs"]
mod footer_tests;
#[cfg(test)]
#[path = "guard_tests.rs"]
mod guard_tests;
#[cfg(test)]
#[path = "numerator_tests.rs"]
mod numerator_tests;
#[cfg(test)]
#[path = "reset_tests.rs"]
mod reset_tests;
#[cfg(test)]
#[path = "warn_absence_tests.rs"]
mod warn_absence_tests;
#[cfg(test)]
#[path = "warn_crossing_tests.rs"]
mod warn_crossing_tests;
#[cfg(test)]
#[path = "window_tests.rs"]
mod window_tests;
