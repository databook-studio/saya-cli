//! First observed safe cause and authoritative terminal persistence.
use super::{CodingError, NativeCodingJob, diagnostics::Fault, readiness};
use saya_agent::{AgentOutput, CancellationReason, ProgressStop};
use saya_harness::coding::coordinator::{ScheduleReport, ScheduleStop};
use saya_types::{
    CodingDiagnosticFault, CodingEventKind, CodingEventV1, CodingFailureClass as Class,
    CodingRunFailure, JobState,
};

mod rank;
mod state;

use state::root;
pub(super) use state::{FailureState, progress_failure};

impl NativeCodingJob {
    fn failures(&self) -> std::sync::MutexGuard<'_, FailureState> {
        self.failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub fn failure(&self) -> Option<CodingRunFailure> {
        self.failures().primary.clone()
    }
    pub(super) fn latch_diagnostic_fault(&self, fault: Fault) {
        use std::sync::atomic::Ordering;
        let mut failures = self.failures();
        failures.diagnostic_fault.get_or_insert(match fault {
            Fault::InvalidRecord => CodingDiagnosticFault::InvalidRecord,
            Fault::UnsafePath => CodingDiagnosticFault::UnsafePath,
            Fault::LockContended => CodingDiagnosticFault::LockContended,
            Fault::Storage => CodingDiagnosticFault::Storage,
            Fault::Recovery => CodingDiagnosticFault::Recovery,
        });
        failures.latch(root(Class::DiagnosticStorage));
        self.observation_failed.store(true, Ordering::Release);
        self.cancellation
            .cancel_with_reason(CancellationReason::OwnerFailure);
    }
    pub(super) fn latch_error(&self, error: &CodingError) {
        if let CodingError::Diagnostic(fault) = error {
            self.latch_diagnostic_fault(*fault);
            return;
        }
        let failure = match error {
            CodingError::Failed(failure) => failure.as_ref().clone(),
            CodingError::OutputLimit { .. } => root(Class::OutputLimit),
            _ => root(Class::Internal),
        };
        let failure = if failure.class == Class::Cancelled {
            root(
                self.cancellation
                    .reason()
                    .map_or(Class::Cancelled, cancellation_class),
            )
        } else {
            failure
        };
        self.failures().latch(failure);
    }
    /// Latches a progress guard's stop. A clean stop returns `Ok` from the
    /// loop, so the caller reads it from the progress handle rather than an
    /// error; a hard stop arrives through [`Self::latch_error`].
    pub(super) fn latch_progress_stop(&self, stop: ProgressStop) {
        self.failures().latch(progress_failure(stop));
    }
    pub(super) fn latch_schedule(&self, report: &ScheduleReport) {
        let mut failures = self.failures();
        // Outcomes are observed at this join; their order is not a claim about
        // the wall-clock order of concurrent child failures.
        for outcome in &report.outcomes {
            failures.latch_schedule_outcome(outcome);
        }
        // A child's own failure is reported in its summary, never as the job's
        // failure, so `ChildFailed` on its own ends nothing here.
        if let Some(stop) = report.stop
            && stop != ScheduleStop::ChildFailed
        {
            failures.latch(root(match stop {
                ScheduleStop::OwnerFailure => Class::MandatoryPersistence,
                ScheduleStop::ChildFailed => Class::Internal,
                ScheduleStop::Cancelled(reason) => cancellation_class(reason),
                ScheduleStop::DependencyBlocked => Class::DependencyBlocked,
                ScheduleStop::Unsettled => Class::EffectsUncertain,
            }));
        }
        if report.effects_uncertain {
            failures.latch(root(Class::EffectsUncertain));
        }
    }
    fn observe_cancellation(&self) -> Option<CancellationReason> {
        let reason = self.cancellation.reason();
        if self.failures().primary.is_none()
            && let Some(reason) = reason
        {
            self.failures().latch(root(cancellation_class(reason)));
        }
        reason
    }
    pub(super) async fn finalize_run(
        &self,
        result: Result<AgentOutput, CodingError>,
    ) -> Result<AgentOutput, CodingError> {
        let stop = saya_agent::CancellationToken::new();
        let finish = async {
            let result = self.finalize_owned(result).await;
            stop.cancel();
            result
        };
        let (result, ()) = tokio::join!(finish, self.watch_cancel(stop.clone()));
        result
    }
    async fn finalize_owned(
        &self,
        result: Result<AgentOutput, CodingError>,
    ) -> Result<AgentOutput, CodingError> {
        if let Err(error) = &result {
            self.latch_error(error);
        }
        if self.failures().primary.is_none()
            && let Some(reason) = self.cancellation.reason()
        {
            self.failures().latch(root(cancellation_class(reason)));
        }
        if result.as_ref().is_ok_and(|output| output.truncated) && self.failures().primary.is_none()
        {
            // A truncated `Ok` is a budget stop: with the progress guards in
            // force, the only remaining budget is a runaway backstop. Name it
            // from the ledger when the cancellation reason carries it.
            let failure = self
                .backstop_failure()
                .await
                .unwrap_or_else(|| root(Class::Backstop));
            self.failures().latch(failure);
        }
        let _operation = self.operation.lock().await;
        // No new operation can prepare while finalization owns this lease.
        // Preserve the earlier root cause before observing any late child.
        if self.active.lock().await.is_some()
            && let Err(error) = self.join().await
        {
            self.latch_error(&error);
        }
        self.observe_cancellation();
        let complete = result.is_ok() && self.failures().primary.is_none();
        // Failure logging is secondary. It must never skip authoritative state.
        if let Err(error) = self.record_observation_boundary("finished", complete).await {
            self.latch_error(&error);
        }
        let cancellation = self.observe_cancellation();
        let (failure, secondary, diagnostic_fault) = {
            let failures = self.failures();
            (
                failures.primary.clone(),
                failures.secondary,
                failures.diagnostic_fault,
            )
        };
        // A final answer with no guard stop is completion, whatever the job
        // changed or committed. A guard stop is a real pause and names its
        // guard; any other failure keeps the failed state.
        let guard_stop = failure
            .as_ref()
            .is_some_and(|failure| failure.agent_id.is_none() && is_guard(failure.class))
            && secondary.is_none()
            && diagnostic_fault.is_none();
        let state = if cancellation == Some(CancellationReason::User) {
            JobState::Cancelled
        } else if guard_stop {
            JobState::Paused
        } else if failure.is_some() {
            JobState::Failed
        } else {
            JobState::Completed
        };
        // `RunTerminated` is the contract for a failed or cancelled run; a
        // guard stop is an ordinary `StateChanged { Paused }` and its cause is
        // kept by the failure state and the outcome block.
        let event = match &failure {
            Some(failure) if matches!(state, JobState::Failed | JobState::Cancelled) => {
                CodingEventKind::RunTerminated {
                    state,
                    failure: failure.clone(),
                    secondary,
                    diagnostic_fault,
                }
            }
            _ => CodingEventKind::StateChanged { state },
        };
        self.lifecycle().enter("terminal");
        if self
            .ledger
            .sequencer()
            .append_event(|sequence| {
                CodingEventV1::new(
                    std::num::NonZeroU64::new(sequence).expect("sequencer allocates nonzero"),
                    self.job_id().clone(),
                    None,
                    None,
                    event,
                )
                .expect("validated terminal event")
            })
            .await
            .is_err()
        {
            return Err(CodingError::TerminalUnconfirmed {
                primary: failure.map(Box::new),
            });
        }
        let (answer, answer_truncated) = match &result {
            Ok(output) => (Some(output.answer.clone()), output.truncated),
            Err(_) => (None, false),
        };
        self.lifecycle().enter("outcome");
        if self
            .record_outcome(state, failure.as_ref(), answer.as_deref(), answer_truncated)
            .await
            .is_err()
        {
            // The terminal state is authoritative; a private outcome artifact
            // that cannot be written must not rewrite it, but it is a storage
            // fault worth naming.
            self.latch_diagnostic_fault(Fault::Storage);
        }
        if let Some(failure) = failure {
            // A clean guard stop returns `Ok` after the model's final turn:
            // the candidate is prepared as for a normal finish, and the stop
            // is still recorded on the terminal event.
            if failure.agent_id.is_none()
                && is_guard(failure.class)
                && secondary.is_none()
                && diagnostic_fault.is_none()
                && let Ok(mut output) = result
            {
                if !self.request.write_paths.is_empty() {
                    self.lifecycle().enter("envelope");
                    output.answer =
                        readiness::final_envelope(&self.candidate_readiness().await).to_string();
                }
                return Ok(output);
            }
            // A native budget/cancellation stop is a bounded-effort result, not
            // successful completion. Adapters also return its typed cause.
            if result.as_ref().is_ok_and(|output| output.truncated)
                && failure.agent_id.is_none()
                && matches!(
                    failure.class,
                    Class::BudgetStopped | Class::Cancelled | Class::LimitReached | Class::Backstop
                )
                && secondary.is_none()
                && diagnostic_fault.is_none()
            {
                return result;
            }
            // Preserve the actionable configured output limit for root failures.
            if failure.agent_id.is_none()
                && failure.class == Class::OutputLimit
                && matches!(result, Err(CodingError::OutputLimit { .. }))
            {
                return result;
            }
            return Err(CodingError::Failed(Box::new(failure)));
        }
        match result {
            Ok(mut output) if !self.request.write_paths.is_empty() => {
                self.lifecycle().enter("envelope");
                output.answer =
                    readiness::final_envelope(&self.candidate_readiness().await).to_string();
                Ok(output)
            }
            result => result,
        }
    }
}
/// The progress guards that stop a run without failing it: the root loop ended
/// with the model's last turn, and the job waits for an operator decision.
fn is_guard(class: Class) -> bool {
    matches!(
        class,
        Class::NoProgress | Class::Repeating | Class::WallClock | Class::Cost
    )
}

fn cancellation_class(reason: CancellationReason) -> Class {
    match reason {
        CancellationReason::Budget(_) => Class::BudgetStopped,
        CancellationReason::OwnerFailure => Class::Internal,
        _ => Class::Cancelled,
    }
}

#[cfg(all(test, unix))]
#[path = "finalization_tests.rs"]
mod finalization_tests;

#[cfg(all(test, unix))]
#[path = "failure_schedule_tests.rs"]
mod schedule_tests;
