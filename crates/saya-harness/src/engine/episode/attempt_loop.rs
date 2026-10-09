use saya_types::{PauseReason, RunPlan};

use crate::workspace::Workspace;

use crate::engine::{sink::EngineEventSink, state::RunState, transitions::TransitionEvent};

use super::{EpisodeDriver, EpisodeError, MAX_EPISODE_ATTEMPTS, deliverables, retry};

impl<'a> EpisodeDriver<'a> {
    pub(super) async fn run_step_inner(
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
            let budget = plan.steps[step].budget.as_ref();
            sink.begin_answering_step(
                step,
                budget.is_some_and(|budget| budget.turns.is_some() || budget.tool_calls.is_some()),
            );
            if let Some(reason) = self.collaborators.cancellation.reason() {
                return self.settle_stop(sink, step, reason).await;
            }
            let result = retry::drive_attempt(self, sink, plan, step, workspace).await;
            // A failed attempt may hold a request the provider never
            // answered: settle it as the empty batch it produced, so the
            // retry's grant check is not refused by stale pending evidence.
            if matches!(
                &result,
                Err(retry::AttemptError::Episode(_))
                    | Err(retry::AttemptError::Gate(EpisodeError::Cancelled { .. }))
            ) {
                sink.settle_failed_request().await;
            }
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
                    if let Some(reason) = self.collaborators.cancellation.reason() {
                        return self.settle_stop(sink, step, reason).await;
                    }
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
}
