use super::{CodingError, NativeCodingJob};
use saya_agent::{AgentError, AgentEventSink, AgentOutput, ProviderError};
use saya_types::CodingRunFailure;
use std::sync::Arc;

impl NativeCodingJob {
    /// Whether the launch scan has landed its summary yet: the fact a test
    /// waits on, and nothing a surface reads — no message is ever held for a
    /// scan.
    #[cfg(test)]
    pub(crate) fn workspace_summary_ready(&self) -> bool {
        self.root_run
            .session()
            .is_some_and(|session| session.summary_ready())
    }

    /// Runs the existing native loop with shared root/child budgets. Always
    /// settles scheduled work before returning, even if the parent fails.
    ///
    /// One job object is one root run: it opens the run, runs the task's one
    /// turn on it and closes it. A message after that is `run_next_turn`'s
    /// (another turn of a run still open), and `resume` stays the between-
    /// process continuation.
    pub async fn run(
        self: &Arc<Self>,
        sink: &dyn AgentEventSink,
    ) -> Result<AgentOutput, CodingError> {
        // The mailbox is taken once, before anything is composed: a second
        // run of this job is `Busy` and touches nothing the first still owns.
        let mailbox = self
            .root_mailbox
            .lock()
            .await
            .take()
            .ok_or(CodingError::Busy)?;
        let monitor_stop = saya_agent::CancellationToken::new();
        // The run is this call's, and it is closed on every exit path — the
        // normal one and a dropped run alike — so a message can never land on
        // a run that ended.
        let _closed = self.root_run.closable();
        let mut guard = RunGuard {
            root: self.cancellation.clone(),
            monitor: monitor_stop.clone(),
            armed: true,
        };
        let work = async {
            self.lifecycle().enter("run_setup");
            // The run's one-time setup and its one turn; the run is closed as
            // soon as that turn ended, so a message now is a finished run.
            let result = self.open_root(sink, mailbox, TurnInputs::default()).await;
            self.root_run.close();
            // The implicit join comes FIRST: a job that delegated without
            // awaiting must still end with its children's work in the candidate,
            // and the owner preparation below refuses while a child is live.
            self.lifecycle().enter("settle");
            self.settle_implicit_join().await;
            // Direct mode: the owner prepares the candidate once the root loop
            // has ended; a preparation failure is not a root failure.
            self.lifecycle().enter("candidate_prepare");
            let result = self.prepare_at_finish(result).await;
            self.lifecycle().enter("finalize");
            if let Err(error) = &result {
                self.latch_error(error);
            }
            if result.is_err() && self.cancellation.reason().is_none() {
                self.cancellation.cancel_with_reason(
                    self.ledger
                        .cancellation_token()
                        .reason()
                        .unwrap_or(saya_agent::CancellationReason::OwnerFailure),
                );
            }
            monitor_stop.cancel();
            result
        };
        // The durable cancel owner stays live through the implicit child join.
        // Both futures remain owned here and stop together if this run is dropped.
        let (result, ()) = tokio::join!(work, self.watch_cancel(monitor_stop.clone()));
        let result = self.finalize_run(result).await;
        self.lifecycle()
            .finish(&self.application.store, self.job_id())
            .await;
        guard.armed = false;
        result
    }
    /// The implicit join at the end of a run: when the root stopped while a
    /// schedule was still live (a job that delegated and never awaited), join
    /// it so the run never ends with live children. Helpers write the shared
    /// working checkout W directly, so a finished child has nothing to
    /// merge and there is no lease to release — the join is the whole of the
    /// end-of-run settle. Nothing happens when no schedule was live.
    ///
    /// `run_finish` states the same join for a prepared run whose turns are
    /// done, so a session's rollover settles its children exactly as `run()`
    /// does.
    pub(super) async fn settle_implicit_join(&self) {
        if self.active.lock().await.is_none() {
            return;
        }
        if let Err(error) = self.join().await {
            self.latch_error(&error);
        }
    }

    pub(super) async fn watch_cancel(&self, stop: saya_agent::CancellationToken) {
        use saya_agent::{BudgetDimension, CancellationReason};
        let owner = self.ledger.cancellation_token();
        let remaining = self.ledger.remaining().await;
        let durable = super::cancel_poll::watch(&self.application.store, self.job_id());
        let reason = tokio::select! {
            biased;
            _=owner.cancelled()=>owner.reason().unwrap_or(CancellationReason::OwnerFailure),
            _=wait_for_deadline(remaining)=>{
                owner.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
                owner.reason().unwrap_or(CancellationReason::Budget(BudgetDimension::ElapsedTime))
            },
            _=stop.cancelled()=>return,
            reason=durable=>reason,
        };
        self.cancellation.cancel_with_reason(reason);
    }
}

async fn wait_for_deadline(deadline: Option<std::time::Duration>) {
    match deadline {
        Some(deadline) => tokio::time::sleep(deadline).await,
        None => std::future::pending().await,
    }
}

/// Keep fixed owner categories; provider text is never sent to presentation or storage.
fn map_root_error(
    error: AgentError,
    configured_max_output_tokens: Option<u32>,
    backstop: Option<CodingRunFailure>,
) -> CodingError {
    match error {
        AgentError::Provider(ProviderError::OutputTruncated { .. }) => CodingError::OutputLimit {
            configured: configured_max_output_tokens,
        },
        // A progress stop names its guard; the loop's own class is only a
        // fallback for a stop that arrived without one.
        AgentError::Progress(stop) => {
            CodingError::Failed(Box::new(super::failure::progress_failure(*stop)))
        }
        error => {
            if let Some(failure) = backstop {
                return CodingError::Failed(Box::new(failure));
            }
            CodingError::Failed(Box::new(CodingRunFailure::root(
                saya_harness::coding::coordinator::classify_agent_failure(&error),
            )))
        }
    }
}

#[cfg(all(test, unix))]
#[path = "parent_tests.rs"]
mod parent_tests;

#[path = "parent_arming.rs"]
mod parent_arming;
#[path = "parent_control.rs"]
mod parent_control;
#[path = "parent_loop.rs"]
mod parent_loop;
#[path = "parent_plugins.rs"]
mod parent_plugins;
#[path = "parent_prepare.rs"]
mod parent_prepare;
#[path = "parent_progress.rs"]
mod parent_progress;
#[path = "parent_run.rs"]
mod parent_run;
#[path = "parent_setup.rs"]
mod parent_setup;
#[path = "parent_turn.rs"]
mod parent_turn;
pub(super) use parent_run::RootRun;
#[path = "parent_turn_limits.rs"]
mod parent_turn_limits;
pub(crate) use parent_turn_limits::RunPlugins;
pub(crate) use parent_turn_limits::TurnInputs;

#[cfg(all(test, unix))]
#[path = "parent_turn_fixture.rs"]
pub(crate) mod parent_turn_fixture;

/// The scripted provider the chat and coding rows drive. It holds no Unix-only
/// call, and the candidates' preflight row — which runs on every host — drives
/// it, so only the Unix-only turn fixtures below stay gated.
#[cfg(test)]
#[path = "parent_turn_provider.rs"]
pub(crate) mod parent_turn_provider;

#[cfg(all(test, unix))]
#[path = "parent_turn_tests.rs"]
mod parent_turn_tests;

#[cfg(all(test, unix))]
#[path = "parent_turn_arming_tests.rs"]
mod parent_turn_arming_tests;

#[cfg(all(test, unix))]
#[path = "parent_turn_arming_elapsed_tests.rs"]
mod parent_turn_arming_elapsed_tests;

#[cfg(all(test, unix))]
#[path = "parent_arming_fixture.rs"]
mod parent_arming_fixture;

struct RunGuard {
    root: saya_agent::CancellationToken,
    monitor: saya_agent::CancellationToken,
    armed: bool,
}
impl Drop for RunGuard {
    fn drop(&mut self) {
        self.monitor.cancel();
        if self.armed {
            self.root
                .cancel_with_reason(saya_agent::CancellationReason::OwnerFailure);
        }
    }
}
