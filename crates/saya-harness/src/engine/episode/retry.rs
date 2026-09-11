//! One bounded attempt: one episode of one step, run through the agent
//! loop, plus the typed progress it leaves behind.
//!
//! An attempt is a fresh brief and a fresh loop — nothing carries over but
//! the plan, the workspace, and the sink. Its failure is classified into a
//! typed `RunFailureCode` for the pause the driver records when the bound
//! is spent, and its boundaries are journaled (`StepStarted` /
//! `StepCompleted` / `StepFailed`) before they are mirrored to the store —
//! the durable record first, the sink's own order for its mirrors.

use saya_agent::{AgentError, AgentOutput, AgentRequest, run_agent_with_sink};
use saya_store::RunStepStatus;
use saya_types::{Deliverable, RunEvent, RunFailureCode, RunPlan};

use crate::workspace::Workspace;

use super::EngineEventSink;
use super::brief;
use super::{EpisodeDriver, EpisodeError, EpisodeRun};

/// How many episodes one step gets, the first included. The engine's
/// bounded step retry: never unbounded, and the bound is spent loudly — a
/// pause with a typed code, never a silent stop.
pub const MAX_EPISODE_ATTEMPTS: usize = 3;

/// Why one attempt failed, and whether the driver may act on it again.
pub(super) enum AttemptError {
    /// The episode's loop failed. Retryable while attempts remain.
    Episode(AgentError),
    /// A gate refused — the brief, a mirror, the machine, or cancellation.
    /// Never retryable: the driver returns it to the caller immediately.
    Gate(EpisodeError),
}

/// The typed cause of an episode's failure. Every `AgentError` the loop can
/// return is an agent-layer failure (the provider, a malformed call, an
/// unrecoverable limit), so they carry the `Provider` code; the safety-gate
/// and connection-config codes belong to layers this loop's errors do not
/// reach. Cancellation never gets here — the driver records it before
/// classifying.
pub(super) fn failure_code(error: &AgentError) -> RunFailureCode {
    match error {
        AgentError::Provider(_) | AgentError::Limit(_) => RunFailureCode::Provider,
        AgentError::InvalidToolCall | AgentError::InvalidHistory => RunFailureCode::Provider,
        // Unreachable in the driver's flow; the match must be total.
        AgentError::Cancelled => RunFailureCode::Provider,
    }
}

/// Runs one episode: a fresh brief (plan state plus the workspace manifest
/// as it stands now), the step's narrowed tools and limits, and the loop.
pub(super) async fn drive_attempt(
    driver: &EpisodeDriver<'_>,
    sink: &EngineEventSink,
    plan: &RunPlan,
    step: usize,
    workspace: &Workspace,
) -> Result<AgentOutput, AttemptError> {
    let Some(spec) = plan.steps.get(step) else {
        return Err(AttemptError::Gate(EpisodeError::OutOfRange {
            step,
            steps: plan.steps.len(),
        }));
    };
    let manifest = brief::manifest(workspace, &driver.bounds)
        .map_err(|source| AttemptError::Gate(EpisodeError::Brief { source }))?;
    let request = AgentRequest {
        prompt: brief::render(plan, step, &manifest),
        profile_names: driver.request.profile_names.clone(),
        model: driver.request.model.clone(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    };
    match run_agent_with_sink(
        driver.collaborators.provider,
        driver.collaborators.tools,
        request,
        brief::definitions(&driver.collaborators.universe, &spec.capabilities),
        brief::limits(&driver.request, spec),
        driver.collaborators.approval,
        sink,
        driver.collaborators.cancellation.clone(),
    )
    .await
    {
        Ok(output) => Ok(output),
        // Cancellation is the user stopping the run, not a step failure:
        // never retried.
        Err(AgentError::Cancelled) => Err(AttemptError::Gate(EpisodeError::Cancelled { step })),
        Err(source) => Err(AttemptError::Episode(source)),
    }
}

/// The durable record first, then the store mirror — the sink's own order
/// for its transitions, kept for the step rows.
async fn mirror(
    run: &EpisodeRun,
    step: usize,
    event: RunEvent,
    status: RunStepStatus,
) -> Result<(), EpisodeError> {
    run.journal
        .append(&event)
        .map_err(|source| EpisodeError::Journal { source })?;
    run.store
        .upsert_step(&run.run_id, step, status)
        .await
        .map_err(|source| EpisodeError::Store { source })
}

/// Journals the episode's start and mirrors the step to `running`.
pub(super) async fn mark_started(run: &EpisodeRun, step: usize) -> Result<(), EpisodeError> {
    mirror(
        run,
        step,
        RunEvent::StepStarted { step },
        RunStepStatus::Running,
    )
    .await
}

/// Journals the step's completion and mirrors the step to `done`.
pub(super) async fn mark_completed(run: &EpisodeRun, step: usize) -> Result<(), EpisodeError> {
    mirror(
        run,
        step,
        RunEvent::StepCompleted { step },
        RunStepStatus::Done,
    )
    .await
}

/// Journals the step's resolved deliverables — the artifact manifest part of
/// the step's completion, recorded just before it. Journal only: the store
/// is metadata, and the manifest is recorded data.
pub(super) fn record_deliverables(
    run: &EpisodeRun,
    step: usize,
    entries: Vec<Deliverable>,
) -> Result<(), EpisodeError> {
    run.journal
        .append(&RunEvent::Deliverables { step, entries })
        .map_err(|source| EpisodeError::Journal { source })
}

/// Journals the episode's failure and mirrors the step to `failed` — the
/// store's step machine allows `failed → running` exactly for the bounded
/// retry the driver performs next.
pub(super) async fn mark_failed(run: &EpisodeRun, step: usize) -> Result<(), EpisodeError> {
    mirror(
        run,
        step,
        RunEvent::StepFailed { step },
        RunStepStatus::Failed,
    )
    .await
}
