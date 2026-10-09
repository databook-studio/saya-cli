use saya_agent::{BudgetDimension, CancellationReason};
use saya_types::RunPlan;

use crate::workspace::Workspace;

use crate::engine::{sink::EngineEventSink, state::RunState};

use super::{EpisodeDriver, EpisodeError};

impl<'a> EpisodeDriver<'a> {
    /// Drives one plan step end to end: `Begin` from an approved run's
    /// first episode, the bounded fresh-brief retry loop, and the boundary
    /// transitions — `Complete` when the plan's last step succeeds, `Pause`
    /// with `StepFailedAfterRetry` when a step's attempts are spent,
    /// `Cancel` when the user stops the episode.
    ///
    /// A controlled deadline and terminal ownership apply when the sink is
    /// attached to this same collaborator cancellation token. Standalone
    /// callers with an unattached legacy sink retain tick-based behavior.
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
        let remaining = match sink.time_remaining() {
            Ok(remaining) => remaining,
            Err(source) if sink.state() == RunState::Approved => {
                return Err(EpisodeError::SinkDiagnostic { source });
            }
            Err(source) => {
                sink.request_owner_failure();
                return self
                    .run_step_inner(sink, plan, step, workspace)
                    .await
                    .map_err(|error| match error {
                        EpisodeError::OwnerFailure { step }
                        | EpisodeError::OwnerFailureDiagnostic { step, .. } => {
                            EpisodeError::OwnerFailureDiagnostic {
                                step,
                                diagnostic: source,
                            }
                        }
                        other => other,
                    });
            }
        };
        let active = self.run_step_inner(sink, plan, step, workspace);
        tokio::pin!(active);
        if !sink.has_cancellation() {
            return active.await;
        }
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
}
