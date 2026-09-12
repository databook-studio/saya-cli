//! `saya run resume <id>` — rebuild a run from its journal and continue it.
//!
//! The engine's `resume` owns the journal replay, the machine advance, and
//! the single-writer lock; the composition root's side is everything the
//! engine cannot derive: the persisted spec (scopes and budgets) and plan,
//! the collaborators built from config the way a fresh run builds them, and
//! the wall clock armed again for the resumed steps. Ctrl-C cancels and
//! exits 130; every typed outcome lands in the shared exit mapping.

use super::exit::{Settled, settle};
use super::{parse_run_id, runs_dir};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use crate::render_run;
use crate::stream_render::TerminalSink;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_harness::engine::{
    EpisodeCollaborators, EpisodeRequest, ResumeRun, RunState, resume as engine_resume,
};
use saya_harness::journal::Journal;
use saya_harness::workspace::Workspace;
use saya_store::{RunStore, SqliteStateStore};
use std::sync::Arc;

pub(super) async fn resume(
    raw_id: &str,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    state: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let run_id = parse_run_id(raw_id)?;
    let dir = runs_dir().join(run_id.as_str());
    let record = RunStore::get_run(state, &run_id)
        .await
        .map_err(|error| format!("run state store refused the lookup: {error}"))?;
    if record.is_none() {
        return crate::commands::output::failure_message(
            2,
            format!("no run with id {run_id}"),
            format,
        );
    }
    let spec = match super::files::load_spec(&dir) {
        Ok(spec) => spec,
        Err(message) => return crate::commands::output::failure_message(2, message, format),
    };
    let plan = match super::files::load_plan(&dir) {
        Ok(plan) => plan,
        Err(message) => return crate::commands::output::failure_message(2, message, format),
    };
    // Runs are headless on resume exactly as fresh: the decider never prompts.
    let cancellation = CancellationToken::default();
    let inputs = super::resume::ResumeInputs {
        spec,
        plan,
        dir,
        state,
    };
    let work = continue_run(runtime, inputs, format, approval, cancellation.clone());
    tokio::pin!(work);
    match tokio::select! {
        result = &mut work => result,
        _ = tokio::signal::ctrl_c() => { cancellation.cancel(); return Ok(130); }
    } {
        Ok(exit) => Ok(exit),
        Err(error) => Err(error),
    }
}

/// What a resume hands the engine: the persisted spec and plan, the run
/// directory, and the store mirror.
struct ResumeInputs<'a> {
    spec: saya_types::RunSpec,
    plan: saya_types::RunPlan,
    dir: std::path::PathBuf,
    state: &'a SqliteStateStore,
}

/// Rebuilds the collaborators and hands the engine the resume inputs. The
/// engine re-derives the machine state from the journal itself and holds the
/// single-writer lock; every failure it returns is mapped here.
async fn continue_run(
    runtime: &RuntimeConfig,
    inputs: ResumeInputs<'_>,
    format: RenderFormat,
    approval: ApprovalPolicy,
    cancellation: CancellationToken,
) -> Result<i32, Box<dyn std::error::Error>> {
    let ResumeInputs {
        spec,
        plan,
        dir,
        state,
    } = inputs;
    let run_id = spec.id.clone();
    let store: Arc<dyn RunStore> = Arc::new(state.clone());
    let workspace = match Workspace::open(&dir.join("workspace")) {
        Ok(workspace) => Arc::new(workspace),
        Err(error) => {
            return crate::commands::output::failure_message(
                3,
                format!("run workspace could not be opened: {error}"),
                format,
            );
        }
    };
    let pieces = match super::assembly::assemble(
        runtime,
        None,
        &dir,
        &spec.scopes,
        workspace.clone(),
        approval,
    )
    .await
    {
        Ok(pieces) => pieces,
        Err(message) => return crate::commands::output::failure_message(3, message, format),
    };
    // The per-step toolsets are prebuilt from the loaded plan, the same
    // fail-closed prebuild a fresh run does: the plan is known before
    // `engine_resume`, so a toolset that cannot be built refuses before
    // the resume runs anything. Each is built from its step's capabilities
    // over the run's shared collaborators, the same way a fresh run builds
    // them.
    let toolsets = super::tools::toolsets(
        super::tools::ToolsetInputs {
            database: &pieces.tools,
            scratch: pieces.scratch.as_ref(),
            fetch: pieces.fetch.as_ref(),
            runner: pieces.runner.as_ref(),
            workspace: &workspace,
            allow_query_data: pieces.allow_query_data,
            cancellation: &cancellation,
        },
        &plan.steps,
    );
    let resumed = ResumeRun {
        run_id: run_id.clone(),
        store: store.clone(),
        plan: plan.clone(),
        workspace: (*workspace).clone(),
        collaborators: EpisodeCollaborators {
            provider: &*pieces.provider,
            approval: &pieces.decider,
            toolsets: &toolsets,
            cancellation: cancellation.clone(),
        },
        request: EpisodeRequest {
            model: pieces.model.clone(),
            profile_names: pieces.profile_names,
            memory_allows_candidate_writes: false,
        },
        bounds: super::assembly::manifest_bounds(),
        wall_clock: spec.budgets.wall_clock,
        token_ceiling: super::budget::token_ceiling(&spec.budgets),
        // The same wallet the fetch-capable steps' executors hold; the engine's
        // resume seeds it with the download spend the run's journal already
        // records, so the budget binds the run, not each invocation. `None`
        // when the run did not approve fetch — the check is inert.
        download_budget: pieces.fetch.as_ref().map(|fetch| fetch.budget.clone()),
        // The resumed run speaks the same wire a fresh run does: the journal
        // carries the event renderer, and the episode's agent events mirror
        // through today's `TerminalEvent` envelope. `engine_resume` opens its
        // own journal from the run directory, so the wire rides the inputs.
        journal_wire: Some(render_run::run_wire(format)),
        agent_stream: Some(Arc::new(TerminalSink::new(format))),
    };
    match engine_resume(&dir, resumed).await {
        Ok(outcome) => match outcome {
            saya_harness::engine::ResumeOutcome::Resumed { state, .. }
            | saya_harness::engine::ResumeOutcome::Settled { state } => {
                // The outcome's state is authoritative; the journal tail
                // supplies only the typed cause of a terminal failure.
                let code = match state {
                    RunState::Failed => super::journal_tail(&Journal::open(&dir)).1,
                    _ => None,
                };
                settle(Settled { state, code }, &run_id, format)
            }
            outcome @ (saya_harness::engine::ResumeOutcome::Unapproved
            | saya_harness::engine::ResumeOutcome::NoRun) => {
                let why = match outcome {
                    saya_harness::engine::ResumeOutcome::Unapproved => {
                        "was never approved, so there is nothing to resume; start a new run \
                         with `saya run`"
                    }
                    _ => "holds no recorded run",
                };
                crate::commands::output::failure_message(2, format!("run {run_id} {why}"), format)
            }
        },
        Err(error) => {
            // Mid-resume failure: the journal tail is the durable authority
            // on where the run ended — the same mapping a fresh run uses.
            let journal = Journal::open(&dir);
            let (tail_state, code) = super::journal_tail(&journal);
            match tail_state {
                RunState::Completed | RunState::Failed | RunState::Cancelled | RunState::Paused => {
                    settle(
                        Settled {
                            state: tail_state,
                            code,
                        },
                        &run_id,
                        format,
                    )
                }
                _ => crate::commands::output::failure_message(3, error.to_string(), format),
            }
        }
    }
}
