//! The fresh-run drive: propose and bind the plan, then execute every step
//! blocking through the engine's drivers.
//!
//! Everything here is covered by Ctrl-C. The plan proposal runs before the
//! sink exists — the wall-clock budget arms at approval, and a plan that
//! never bound stops the run by cause with nothing to resume. The bound
//! plan is persisted before approval so a crash between the two leaves a
//! resumable record either way.

use super::assembly;
use super::host::HostRun;
use super::mode::RunApproval;
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_harness::engine::EngineEventSink;
use saya_harness::journal::Journal;
use saya_harness::workspace::Workspace;
use saya_store::{RunStore, SqliteStateStore};
use saya_types::{RunId, RunSpec};
use std::sync::Arc;

mod execute;
mod prepare;

enum PreparedDrive {
    Terminal(i32),
    Ready(Box<PreparedState>),
}

struct PreparedState {
    run_id: RunId,
    format: RenderFormat,
    pieces: assembly::Pieces,
    workspace: Arc<Workspace>,
    plan: saya_types::RunPlan,
    toolsets: Vec<saya_harness::engine::StepToolset>,
    sink: EngineEventSink,
    store: Arc<dyn RunStore>,
    journal: Journal,
    cancellation: saya_agent::CancellationToken,
}

/// What a fresh run's drive needs, the way the resume's `ResumeInputs`
/// bundles its own: the spec, the claimed directory, the store mirror, and
/// the composition inputs the engine cannot derive — config, rendering, the
/// admitted approval (the boundary's type, never a raw mode), the run's
/// stated scope words, and the host's observers and cancellation.
pub(super) struct DriveInputs<'a> {
    pub(super) spec: &'a RunSpec,
    pub(super) run_dir: saya_harness::run_dir::RunDir,
    pub(super) state: &'a SqliteStateStore,
    pub(super) runtime: &'a RuntimeConfig,
    pub(super) format: RenderFormat,
    /// The run's admitted mode: every entry point into a run passes the
    /// boundary's own admission (`mode.rs`), so bypass cannot reach the
    /// composition here — or anywhere below it.
    pub(super) approval: RunApproval,
    /// The run's stated scopes as the grammar's words — the frozen decider's
    /// seeds, and the carried words (`sql:`) the approval payload records.
    pub(super) allow_tokens: Vec<String>,
    pub(super) host: HostRun<'a>,
}

/// Proposes and binds the plan, then drives every step. Everything here is
/// covered by Ctrl-C. `plan_approval` is the surface the bound plan's one
/// approval decision flows through.
///
/// The run wire is attached here, not rendered by hand: the journal carries
/// the wire, so every lifecycle and step event is rendered from the journal's
/// own write (in journal order, never duplicated), and the engine sink
/// forwards the episode's agent events through today's `TerminalEvent`
/// envelope in the caller's format. The two tags share the stream by design —
/// see `crate::render_run` for why the benchmark's wire stays intact.
pub(super) async fn drive(inputs: DriveInputs<'_>) -> Result<i32, Box<dyn std::error::Error>> {
    let prepared = prepare::prepare(inputs).await?;
    execute::execute(prepared).await
}
