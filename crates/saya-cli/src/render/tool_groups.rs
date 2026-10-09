//! The shared tool-call grouper and shaper: a sequence of [`AgentEvent`] in,
//! a sequence of groups out, and a shaper turning a group into summary text.
//!
//! Pure and deterministic: no clock, no environment reads, no I/O. Both
//! adapters (the piped text renderer and the TUI transcript) consume this one
//! operation; the per-adapter wiring is a later slice.

use std::collections::HashMap;

use saya_agent::{AgentEvent, ToolEffect, read_only_permits};
include!("tool_groups/model.rs");
include!("tool_groups/collect.rs");
include!("tool_groups/call_lines.rs");
include!("tool_groups/key_labels.rs");
include!("tool_groups/summaries.rs");
#[path = "tool_groups_tests.rs"]
#[cfg(test)]
mod tests;
