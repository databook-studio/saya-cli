//! What a non-headless host drives the fresh-run path with.
//!
//! The headless `saya run` observes itself through the process stream: the
//! journal's wire renders each lifecycle event, and the episode's agent
//! events mirror through the terminal sink. A host that owns the terminal —
//! the TUI's run panel — injects its own observers here and the drive path
//! forwards to them instead. Nothing else changes: the same spec, claim,
//! plan, approval gate, and exit mapping run underneath, so a panel-driven
//! run and a headless run cannot drift into two paths.
//!
//! The plan-approval surface rides here too: the host's modal answers it
//! (the M1-10 channel, `ask.rs`'s shape) rather than any stdin prompt —
//! approval stays the engine's one gate, shown once.

use saya_agent::{AgentEventSink, CancellationToken};
use saya_harness::journal::JournalWire;
use std::sync::Arc;

use super::approval::PlanApproval;

/// The host's injection into one fresh run. `None` observers keep the
/// headless defaults — the process stream — so the headless call sites
/// change nothing.
pub(crate) struct HostRun<'a> {
    /// Where each journaled [`RunEvent`] is reported, fired from the
    /// journal's own write (the same stream the durable record is).
    pub(crate) journal_wire: Option<JournalWire>,
    /// Where the episode's agent events are forwarded.
    pub(crate) agent_stream: Option<Arc<dyn AgentEventSink>>,
    /// The host's active connection profile — what a nested child's
    /// `--profile` forwards. `None` keeps the resolved default.
    pub(crate) profile: Option<&'a String>,
    /// The plan-approval surface; headless runs pre-authorize their scopes.
    pub(crate) plan_approval: &'a PlanApproval,
    /// The token that stops the run. The headless path's Ctrl-C cancels its
    /// own fresh token; a host hands its panel's token in.
    pub(crate) cancellation: CancellationToken,
}

/// What one fresh run is asked to do: the id its directory and journal carry,
/// the goal, and the approved scopes and budgets the CLI or the panel parsed.
/// One shape for both entries (`saya run` and the panel's), so the two
/// surfaces cannot drift on what a run consists of.
pub(crate) struct RunRequest {
    pub(crate) run_id: saya_types::RunId,
    pub(crate) goal: Option<String>,
    pub(crate) allow: Vec<String>,
    pub(crate) budget: Vec<String>,
}
