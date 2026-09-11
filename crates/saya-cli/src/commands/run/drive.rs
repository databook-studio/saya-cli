//! The fresh-run drive: propose and bind the plan, then execute every step
//! blocking through the engine's drivers.
//!
//! Everything here is covered by Ctrl-C. The plan proposal runs before the
//! sink exists — the wall-clock budget arms at approval, and a plan that
//! never bound stops the run by cause with nothing to resume. The bound
//! plan is persisted before approval so a crash between the two leaves a
//! resumable record either way.

use super::exit::Settled;
use super::{assembly, exit, files};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use crate::render_run;
use crate::stream_render::TerminalSink;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_harness::engine::{
    EngineEventSink, EpisodeCollaborators, EpisodeDriver, EpisodeRequest, EpisodeRun, PlanDriver,
    PlanRequest, RunState, TransitionEvent,
};
use saya_harness::journal::Journal;
use saya_harness::workspace::Workspace;
use saya_store::{RunStore, SqliteStateStore};
use saya_types::{Budgets, RunSpec};
use std::sync::Arc;

/// Proposes and binds the plan, then drives every step. Everything here is
/// covered by Ctrl-C.
///
/// The run wire is attached here, not rendered by hand: the journal carries
/// the wire, so every lifecycle and step event is rendered from the journal's
/// own write (in journal order, never duplicated), and the engine sink
/// forwards the episode's agent events through today's `TerminalEvent`
/// envelope in the caller's format. The two tags share the stream by design —
/// see `crate::render_run` for why the benchmark's wire stays intact.
pub(super) async fn drive(
    spec: &RunSpec,
    run_dir: saya_harness::run_dir::RunDir,
    state: &SqliteStateStore,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    cancellation: CancellationToken,
) -> Result<i32, Box<dyn std::error::Error>> {
    let run_id = spec.id.clone();
    let store: Arc<dyn RunStore> = Arc::new(state.clone());
    let journal = render_run::wired_journal(Journal::open(run_dir.root()), format);
    let workspace = match Workspace::open(run_dir.workspace()) {
        Ok(workspace) => Arc::new(workspace),
        Err(error) => {
            return exit::connection_failure(
                format!("run workspace could not be opened: {error}"),
                format,
            );
        }
    };
    let pieces = match assembly::assemble(runtime, &spec.scopes, workspace.clone(), approval).await
    {
        Ok(pieces) => pieces,
        Err(message) => return exit::connection_failure(message, format),
    };
    let plan = match PlanDriver::new(
        &*pieces.provider,
        PlanRequest {
            model: pieces.model.clone(),
            run_goal: spec.goal.clone(),
        },
    )
    .propose(&spec.scopes, &spec.budgets)
    .await
    {
        Ok(plan) => plan,
        Err(error) => {
            // The plan never bound: the run stops by cause before it began,
            // staying `planned` — there is nothing to resume, and the exit
            // says the provider layer failed.
            return crate::commands::output::failure_message(
                5,
                format!("run {run_id} could not bind a plan: {error}"),
                format,
            );
        }
    };
    // A step whose budget is unset inherits the run's budgets as its
    // ceilings (the StepSpec contract's layering rule); the persisted plan
    // is the layered one a resume replays.
    let plan = bind_step_budgets(plan, &spec.budgets);
    if let Err(error) = files::persist_plan(run_dir.root(), &plan) {
        return exit::connection_failure(
            format!("run plan could not be persisted: {error}"),
            format,
        );
    }
    let sink = EngineEventSink::new(
        run_id.clone(),
        RunState::Planned,
        journal.clone(),
        store.clone(),
        spec.budgets.wall_clock,
        std::time::Instant::now,
    )
    .with_agent_stream(Arc::new(TerminalSink::new(format)));
    if let Err(error) = sink.record(TransitionEvent::Approve).await {
        return exit::connection_failure(
            format!("run approval could not be recorded: {error}"),
            format,
        );
    }
    let driver = EpisodeDriver::new(
        EpisodeCollaborators {
            provider: &*pieces.provider,
            tools: &pieces.tools,
            approval: &pieces.decider,
            universe: pieces.universe,
            cancellation: cancellation.clone(),
        },
        EpisodeRun {
            run_id: run_id.clone(),
            store: store.clone(),
            journal,
        },
        EpisodeRequest {
            model: pieces.model.clone(),
            profile_names: pieces.profile_names,
            memory_allows_candidate_writes: false,
        },
        assembly::manifest_bounds(),
    );
    match drive_steps(&driver, &sink, &plan, &workspace).await {
        Ok(()) => exit::settle(
            Settled {
                state: sink.state(),
                code: None,
            },
            &run_id,
            format,
        ),
        Err(error) => exit::settle(
            Settled {
                state: sink.state(),
                code: failure_code_of(&error),
            },
            &run_id,
            format,
        ),
    }
}

/// Drives the plan's steps in order; the first error stops the loop.
async fn drive_steps(
    driver: &EpisodeDriver<'_>,
    sink: &EngineEventSink,
    plan: &saya_types::RunPlan,
    workspace: &saya_harness::workspace::Workspace,
) -> Result<(), saya_harness::engine::EpisodeError> {
    for step in 0..plan.steps.len() {
        driver.run_step(sink, plan, step, workspace).await?;
    }
    Ok(())
}

/// The typed cause an episode error carries, when it carries one.
fn failure_code_of(
    error: &saya_harness::engine::EpisodeError,
) -> Option<saya_types::RunFailureCode> {
    match error {
        saya_harness::engine::EpisodeError::StepExhausted { code, .. } => Some(*code),
        _ => None,
    }
}

/// A step whose budget is unset inherits the run's budgets as its ceilings
/// (the `StepSpec` contract's layering rule). The engine's brief reads only
/// the step budget, so the composition root applies the inheritance when
/// binding: the persisted plan is the layered one a resume replays.
fn bind_step_budgets(plan: saya_types::RunPlan, run: &Budgets) -> saya_types::RunPlan {
    let mut plan = plan;
    for step in &mut plan.steps {
        if step.budget.is_none() {
            step.budget = Some(run.clone());
        }
    }
    plan
}
