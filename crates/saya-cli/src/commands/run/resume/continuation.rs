use super::super::exit::{Settled, settle};
use super::super::mode::RunApproval;
use super::uncertainty;
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use crate::render_run;
use crate::stream_render::TerminalSink;
use saya_agent::CancellationToken;
use saya_harness::engine::{
    EpisodeCollaborators, EpisodeRequest, IncompletePolicy, ResumeOutcome, ResumeRun, RunState,
    resume as engine_resume,
};
use saya_harness::journal::Journal;
use saya_harness::workspace::Workspace;
use saya_store::{RunStore, SqliteStateStore};
use std::sync::Arc;

/// What a resume hands the engine: the persisted spec and plan, the run
/// directory, and the store mirror.
pub(super) struct ResumeInputs<'a> {
    pub(super) spec: saya_types::RunSpec,
    pub(super) plan: saya_types::RunPlan,
    pub(super) dir: std::path::PathBuf,
    pub(super) state: &'a SqliteStateStore,
}

/// Rebuilds the collaborators and hands the engine the resume inputs. The
/// engine re-derives the machine state from the journal itself and holds the
/// single-writer lock; every failure it returns is mapped here. The approval
/// is the boundary's admitted mode — never a raw policy.
pub(super) async fn continue_run(
    runtime: &RuntimeConfig,
    inputs: ResumeInputs<'_>,
    format: RenderFormat,
    approval: RunApproval,
    cancellation: CancellationToken,
    retry_incomplete: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let ResumeInputs {
        spec,
        plan,
        dir,
        state,
    } = inputs;
    let run_id = spec.id.clone();
    let store: Arc<dyn RunStore> = Arc::new(state.clone());
    // The journal is the authority a resume re-grants from: the approved
    // capabilities come from the journal's `PlanApproved` payload when it
    // states scopes — words or the stated empty set of a `--allow none`
    // run, exactly what the original approval carried, no more, no fewer —
    // and the persisted spec stands in only where the journal predates the
    // payload field. A `spec.json` edited between invocations cannot widen
    // what the journal stated: the grant is the journal's, never the
    // file's. The payload's words are also the resumed decider's seeds —
    // the frozen policy holds exactly what the journal approved, and a
    // pre-payload journal seeds nothing.
    let grants = super::super::grants::journal_grants(&dir, &spec.scopes)?;
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
    let pieces = match super::super::assembly::assemble(
        runtime,
        None,
        &dir,
        &grants.capabilities,
        workspace.clone(),
        approval,
        &grants.tokens,
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
    let toolsets = super::super::tools::toolsets(
        super::super::tools::ToolsetInputs {
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
        incomplete_policy: if retry_incomplete {
            IncompletePolicy::Retry
        } else {
            IncompletePolicy::Conservative
        },
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
        bounds: super::super::assembly::manifest_bounds(),
        wall_clock: spec.budgets.wall_clock,
        token_ceiling: super::super::budget::token_ceiling(&spec.budgets),
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
            ResumeOutcome::Resumed { state, .. } | ResumeOutcome::Settled { state } => {
                // The outcome's state is authoritative; the journal tail
                // supplies only the typed cause of a terminal failure.
                let code = match state {
                    RunState::Failed => super::super::journal_tail(&Journal::open(&dir)).1,
                    _ => None,
                };
                settle(Settled { state, code }, &run_id, format)
            }
            outcome @ (ResumeOutcome::Unapproved | ResumeOutcome::NoRun) => {
                let why = match outcome {
                    ResumeOutcome::Unapproved => {
                        "was never approved, so there is nothing to resume; start a new run \
                         with `saya run`"
                    }
                    _ => "holds no recorded run",
                };
                crate::commands::output::failure_message(2, format!("run {run_id} {why}"), format)
            }
            ResumeOutcome::IncompleteEffects {
                step,
                goal,
                effects,
            } => uncertainty::emit(&run_id, step + 1, &goal, effects, format),
        },
        Err(error) => {
            // Mid-resume failure: the journal tail is the durable authority
            // on where the run ended — the same mapping a fresh run uses.
            let journal = Journal::open(&dir);
            let (tail_state, code) = super::super::journal_tail(&journal);
            if matches!(
                &error,
                saya_harness::engine::ResumeError::CountEvidence { .. }
            ) {
                return crate::commands::output::failure_message(
                    3,
                    format!("run {run_id} cannot resume with unknown step counts: {error}"),
                    format,
                );
            }
            if matches!(
                &error,
                saya_harness::engine::ResumeError::Episode {
                    source: saya_harness::engine::EpisodeError::BudgetStoppedDiagnostic { .. }
                }
            ) {
                return crate::commands::output::failure_message(
                    6,
                    format!("run {run_id} paused after its stop settled: {error}"),
                    format,
                );
            }
            if matches!(
                &error,
                saya_harness::engine::ResumeError::Episode {
                    source: saya_harness::engine::EpisodeError::OwnerFailureDiagnostic { .. }
                } | saya_harness::engine::ResumeError::Episode {
                    source: saya_harness::engine::EpisodeError::SinkDiagnostic { .. }
                }
            ) {
                return crate::commands::output::failure_message(
                    3,
                    format!("run {run_id} stopped after an owner error: {error}"),
                    format,
                );
            }
            if matches!(
                &error,
                saya_harness::engine::ResumeError::Episode {
                    source: saya_harness::engine::EpisodeError::CancelledDiagnostic { .. }
                } | saya_harness::engine::ResumeError::Episode {
                    source: saya_harness::engine::EpisodeError::CancelPersistenceFailed { .. }
                } | saya_harness::engine::ResumeError::Episode {
                    source: saya_harness::engine::EpisodeError::BudgetPersistenceFailed { .. }
                } | saya_harness::engine::ResumeError::Episode {
                    source: saya_harness::engine::EpisodeError::Transition { .. }
                }
            ) && matches!(
                tail_state,
                RunState::Completed | RunState::Failed | RunState::Cancelled | RunState::Paused
            ) {
                return super::super::exit::settle_diagnostic(
                    Settled {
                        state: tail_state,
                        code,
                    },
                    &error,
                    &run_id,
                    format,
                );
            }
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
