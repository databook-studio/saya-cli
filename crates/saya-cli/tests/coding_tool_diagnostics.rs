//! Durable root+child tool diagnostics: owner-boundary regressions for the
//! proposed private `tool-events.jsonl` journal (metadata only) written from
//! actual native root/child tool execution. These tests drive a scripted real
//! run (real job, real children, real checkouts) with `NoopEventSink` and then
//! read the journal the production run must leave behind, so the proof never
//! depends on the events the sink forwarded or on `ToolCompleted`'s arbitrary
//! summary.
//!
#![cfg(unix)]

#[path = "coding_tool_diagnostics/bounds.rs"]
mod bounds;
#[path = "coding_tool_diagnostics/details.rs"]
mod details;
#[path = "coding_tool_diagnostics/journal.rs"]
mod journal;
#[path = "coding_tool_diagnostics/provider.rs"]
mod provider;
#[path = "coding_tool_diagnostics/run.rs"]
mod run;
#[path = "coding_application/support.rs"]
#[allow(dead_code)] // shared test harness: this file uses only part of it
mod support;
#[path = "coding_tool_diagnostics/transcript.rs"]
mod transcript;
