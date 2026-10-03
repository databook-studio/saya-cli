//! The episode driver: one plan step, driven end to end through the agent
//! loop.
//!
//! Per step the driver builds the brief (the plan state plus the workspace
//! manifest), runs the step's own toolset — the executor and definitions the
//! composition root built from the step's capabilities, so a tool outside
//! them is hidden, not advertised-and-refused — turns the step's budget into
//! `AgentLimits` with no environment input (a run is reproducible from its
//! spec and config alone, plan G3), pins learning off explicitly (DESIGN
//! §5.8), and retries a failing episode a bounded number of times with a
//! fresh brief. When the bound is spent the run pauses with a typed failure
//! code; it never retries unbounded and never fails silently.
//!
//! The lifecycle transitions the driver records through the sink: `Begin`
//! (an approved run starts executing with its first episode), `Complete`
//! (the plan's last step succeeded), `Pause` (`StepFailedAfterRetry`), and
//! `Cancel` (the user stopped the episode). Mid-run steps need no run-level
//! transition — the steps' `StepStarted` journal events tell that story.

mod brief;
mod contract;
mod deliverables;
mod retry;

pub use contract::{
    EpisodeCollaborators, EpisodeError, EpisodeRequest, EpisodeRun, ManifestBounds, StepToolset,
};
pub use retry::MAX_EPISODE_ATTEMPTS;

use saya_agent::{BudgetDimension, CancellationReason};
use saya_types::{PauseReason, RunPlan};

use crate::workspace::Workspace;

use super::sink::EngineEventSink;
use super::state::RunState;
use super::transitions::TransitionEvent;

/// Drives one plan step: the brief, the narrowed tools, the step budget,
/// and the bounded fresh-brief retries, through the engine's event sink.
pub struct EpisodeDriver<'a> {
    collaborators: EpisodeCollaborators<'a>,
    run: EpisodeRun,
    request: EpisodeRequest,
    bounds: contract::ManifestBounds,
}

impl<'a> EpisodeDriver<'a> {
    /// Assembles the driver from the pieces the composition root owns.
    pub fn new(
        collaborators: EpisodeCollaborators<'a>,
        run: EpisodeRun,
        request: EpisodeRequest,
        bounds: contract::ManifestBounds,
    ) -> Self {
        Self {
            collaborators,
            run,
            request,
            bounds,
        }
    }

    /// Drives one plan step end to end: `Begin` from an approved run's
    /// first episode, the bounded fresh-brief retry loop, and the boundary
    /// transitions — `Complete` when the plan's last step succeeds, `Pause`
    /// with `StepFailedAfterRetry` when a step's attempts are spent,
    /// `Cancel` when the user stops the episode.
    ///
    /// Gate refusals (brief, journal, store, machine) are never retried and
    /// never silent: they return immediately, with the run's state as the
    /// sink last advanced it. Retryable means the episode itself failed;
    /// every attempt begins with a fresh brief and a fresh loop.
    pub async fn run_step(
        &self,
        sink: &EngineEventSink,
        plan: &RunPlan,
        step: usize,
        workspace: &Workspace,
    ) -> Result<(), EpisodeError> {
        let remaining = sink.time_remaining();
        let active = self.run_step_inner(sink, plan, step, workspace);
        tokio::pin!(active);
        if remaining.is_some_and(|remaining| remaining.is_zero()) {
            self.collaborators
                .cancellation
                .cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
        }
        if let Some(remaining) = remaining.filter(|remaining| !remaining.is_zero()) {
            tokio::select! {
                biased;
                _ = tokio::time::sleep(remaining) => {
                    self.collaborators.cancellation.cancel_with_reason(CancellationReason::Budget(
                        BudgetDimension::ElapsedTime,
                    ));
                    active.await
                }
                result = &mut active => result,
            }
        } else {
            active.await
        }
    }

    async fn run_step_inner(
        &self,
        sink: &EngineEventSink,
        plan: &RunPlan,
        step: usize,
        workspace: &Workspace,
    ) -> Result<(), EpisodeError> {
        if step >= plan.steps.len() {
            return Err(EpisodeError::OutOfRange {
                step,
                steps: plan.steps.len(),
            });
        }
        match sink.state() {
            // An approved run begins executing with its first episode.
            RunState::Approved if step == 0 => {
                sink.record(TransitionEvent::Begin)
                    .await
                    .map_err(|source| EpisodeError::Transition { source })?;
            }
            RunState::Approved => {
                return Err(EpisodeError::NotRunnable {
                    step,
                    state: sink.state(),
                });
            }
            // Mid-run steps need no run-level transition; each episode's
            // `StepStarted` journal event marks its start.
            RunState::Executing => {}
            state => return Err(EpisodeError::NotRunnable { step, state }),
        }
        if let Some(reason) = self.collaborators.cancellation.reason() {
            return self.settle_stop(sink, step, reason).await;
        }
        let mut attempt = 0;
        loop {
            if self.collaborators.cancellation.is_cancelled() {
                return self
                    .settle_stop(
                        sink,
                        step,
                        self.collaborators.cancellation.reason().unwrap(),
                    )
                    .await;
            }
            attempt += 1;
            retry::mark_started(&self.run, step).await?;
            if let Some(reason) = self.collaborators.cancellation.reason() {
                return self.settle_stop(sink, step, reason).await;
            }
            let result = retry::drive_attempt(self, sink, plan, step, workspace).await;
            if let Some(reason) = self.collaborators.cancellation.reason() {
                return self.settle_stop(sink, step, reason).await;
            }
            match result {
                Ok(_) => {
                    // The step's declared deliverables become the artifact
                    // manifest at completion: resolved against the workspace
                    // and recorded just before the completion itself, so a
                    // crash between the two leaves the step in flight — a
                    // resume re-runs it rather than reporting a manifest for
                    // a completion that never recorded. A refusal is a gate
                    // error: never retried, never silent.
                    let deliverables =
                        deliverables::resolve(workspace, &plan.steps[step], &self.bounds)
                            .map_err(|source| EpisodeError::Deliverables { step, source })?;
                    if !deliverables.is_empty() {
                        retry::record_deliverables(&self.run, step, deliverables)?;
                    }
                    retry::mark_completed(&self.run, step).await?;
                    if let Some(reason) = self.collaborators.cancellation.reason() {
                        return self.settle_stop(sink, step, reason).await;
                    }
                    if step + 1 == plan.steps.len() {
                        sink.record(TransitionEvent::Complete)
                            .await
                            .map_err(|source| EpisodeError::Transition { source })?;
                    }
                    return Ok(());
                }
                Err(retry::AttemptError::Gate(EpisodeError::Cancelled { step })) => {
                    sink.record(TransitionEvent::Cancel)
                        .await
                        .map_err(|source| EpisodeError::Transition { source })?;
                    return Err(EpisodeError::Cancelled { step });
                }
                // A gate refusal is never retried: the durable record did
                // not advance, and proceeding would be unrecorded work.
                Err(retry::AttemptError::Gate(gate)) => return Err(gate),
                Err(retry::AttemptError::Episode(source)) => {
                    retry::mark_failed(&self.run, step).await?;
                    if attempt >= MAX_EPISODE_ATTEMPTS {
                        sink.record(TransitionEvent::Pause(PauseReason::StepFailedAfterRetry))
                            .await
                            .map_err(|source| EpisodeError::Transition { source })?;
                        return Err(EpisodeError::StepExhausted {
                            step,
                            attempts: attempt,
                            code: retry::failure_code(&source),
                        });
                    }
                }
            }
        }
    }

    async fn settle_stop(
        &self,
        sink: &EngineEventSink,
        step: usize,
        reason: CancellationReason,
    ) -> Result<(), EpisodeError> {
        match reason {
            CancellationReason::User => {
                sink.record(TransitionEvent::Cancel)
                    .await
                    .map_err(|source| EpisodeError::Transition { source })?;
                Err(EpisodeError::Cancelled { step })
            }
            CancellationReason::Budget(dimension) => {
                let reason = if dimension == BudgetDimension::ElapsedTime {
                    PauseReason::WallClockExceeded
                } else {
                    PauseReason::BudgetExhausted
                };
                sink.record(TransitionEvent::Pause(reason))
                    .await
                    .map_err(|source| EpisodeError::Transition { source })?;
                Err(EpisodeError::BudgetStopped { step, reason })
            }
            CancellationReason::OwnerFailure => Err(EpisodeError::OwnerFailure { step }),
        }
    }
}
