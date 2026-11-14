use super::{CodingError, NativeCodingJob, convergence::ConvergenceSink};
use crate::agent::hooks::{HookApprovalGate, HookCaller, HookGate, Hooks};
use crate::agent::{provider, tools::coding::RootTools};
use saya_agent::{
    AgentError, AgentEventSink, AgentOutput, AgentRequest, AgentRunControl, ApprovalPolicy,
    ProgressCheck, ProgressHandle, ProviderError, run_agent_with_control,
};
use saya_types::CodingRunFailure;
use std::sync::{Arc, Mutex};

impl NativeCodingJob {
    /// Runs the existing native loop with shared root/child budgets. Always
    /// settles scheduled work before returning, even if the parent fails.
    pub async fn run(
        self: &Arc<Self>,
        sink: &dyn AgentEventSink,
    ) -> Result<AgentOutput, CodingError> {
        let mailbox = self
            .root_mailbox
            .lock()
            .await
            .take()
            .ok_or(CodingError::Busy)?;
        let monitor_stop = saya_agent::CancellationToken::new();
        let mut guard = RunGuard {
            root: self.cancellation.clone(),
            monitor: monitor_stop.clone(),
            armed: true,
        };
        let work = async {
            self.lifecycle().enter("run_setup");
            let result = self.run_root(sink, mailbox).await;
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
    async fn settle_implicit_join(&self) {
        if self.active.lock().await.is_none() {
            return;
        }
        if let Err(error) = self.join().await {
            self.latch_error(&error);
        }
    }
    async fn run_root(
        self: &Arc<Self>,
        sink: &dyn AgentEventSink,
        mailbox: saya_agent::AgentMailbox,
    ) -> Result<AgentOutput, CodingError> {
        // A direct-mode coordinator owns one working checkout for the whole
        // run; `RootTools::build` ensures it (resume re-ensures it) and composes
        // the toolset. Delegate-only jobs create nothing.
        self.record_provider_binding().map_err(|_| {
            CodingError::Failed(Box::new(CodingRunFailure::root(
                saya_types::CodingFailureClass::MandatoryPersistence,
            )))
        })?;
        let config = &self.application.runtime.resolved;
        self.lifecycle().enter("provider");
        let provider = provider::build(&config.ai, &self.application.runtime.secret_resolver())
            .map_err(|_| {
                CodingError::Failed(Box::new(CodingRunFailure::root(
                    saya_types::CodingFailureClass::ProviderError,
                )))
            })?;
        self.lifecycle().enter("tools");
        let (tools, built_checkout) = RootTools::build(self).await?;
        let tools = Arc::new(tools);
        // The run's own working checkout, resolved once: `RootTools::build` has
        // just ensured it and returns the use-time verification it composed the
        // toolset with, so every checkpoint this run takes is of this one
        // directory without a second inventory of it. A read-only surface has
        // no checkout to take; a checkout that cannot be resolved takes no
        // checkpoint, and that refusal is recorded rather than failing the run.
        let checkout = match built_checkout {
            Some(verified) => Some(verified),
            None => match self.verified_root_checkout().await {
                Ok(verified) => verified.map(std::sync::Arc::new),
                Err(error) => {
                    self.record_capture_refusal(&error);
                    None
                }
            },
        };
        // The user's own hooks, over the run's working checkout — the one
        // directory this run's tools work in. A job whose checkout cannot be
        // resolved runs no hook, the same way it takes no checkpoint. The mode
        // is `bypass`: this root decides its own calls through its own
        // decider, so the host lane a hook runs in is the lane this run
        // already has.
        let hooks = checkout.as_ref().and_then(|verified| {
            Hooks::compose(
                &self.application.runtime,
                ApprovalPolicy::Bypass,
                std::path::Path::new(verified.path().as_str()),
                HookCaller::Job(self.job_id().as_str().to_owned()),
                self.cancellation.clone(),
            )
        });
        // The advisory readiness may be computed while the model streams; the
        // precompute itself decides whether this run's shape allows it.
        self.plan_readiness_precompute(hooks.is_some());
        // The job root's start hook, once; its bounded stdout rides every turn.
        let session_context = match hooks.as_ref() {
            Some(hooks) => hooks.session_start().await,
            None => None,
        };
        let executor: Arc<dyn saya_agent::ToolExecutor> = match hooks.as_ref() {
            Some(hooks) => Arc::new(
                HookGate::new(
                    Arc::clone(hooks),
                    Arc::clone(&tools) as Arc<dyn saya_agent::ToolExecutor>,
                )
                .with_session_context(session_context),
            ),
            None => Arc::clone(&tools) as Arc<dyn saya_agent::ToolExecutor>,
        };
        // A `PermissionRequest` hook can only refuse an ask this root would make.
        let approval_gate = hooks.as_ref().map(|hooks| {
            HookApprovalGate::new(
                Arc::clone(hooks),
                Arc::clone(&tools) as Arc<dyn saya_agent::ApprovalDecider>,
            )
        });
        let approver: &dyn saya_agent::ApprovalDecider = match &approval_gate {
            Some(gate) => gate,
            None => tools.as_ref(),
        };
        let root_budget = self.ledger.root_view();
        let observed = super::observations::NativeSink {
            job: self,
            downstream: sink,
        };
        let watched = ConvergenceSink::new(self, &observed);
        // The turn-boundary checkpoint rides outside the stall watcher: it
        // observes the same events and changes nothing about them.
        let checkpointed = super::rewind::RewindSink::new(self, &watched, checkout);
        self.record_observation_boundary("started", false).await?;
        // The loop publishes progress here after every turn; a clean stop
        // returns `Ok`, so the stop itself travels on this handle. Each turn
        // that reports a check is also appended to `progress.jsonl` as it
        // happens, so `saya job show` sees the streak while the job runs.
        let checks: Arc<Mutex<Vec<ProgressCheck>>> = Arc::default();
        let observed_checks = Arc::clone(&checks);
        let progress_records = self.records.clone();
        let progress_lifecycle = self.lifecycle_arc();
        let progress = ProgressHandle::new().with_observer(Arc::new(move |snapshot| {
            let Some(check) = snapshot.report.check else {
                return;
            };
            let Ok(mut checks) = observed_checks.lock() else {
                return;
            };
            // A turn boundary can publish twice (the final turn is published
            // by its own end and again with the stop); record it once.
            if checks.last().is_some_and(|last| last.turn == check.turn) {
                return;
            }
            checks.push(check);
            let started = std::time::Instant::now();
            super::progress_report::record(&progress_records, &snapshot.report);
            progress_lifecycle.note_elapsed("progress_persist", started.elapsed());
        }));
        let control = AgentRunControl::new(
            super::limits::agent_limits(config),
            super::limits::answering_limits(config),
            approver,
            &checkpointed,
            self.cancellation.clone(),
        )
        .with_actor_correlation(super::create::actor_correlation_digest(&[
            "root",
            self.job_id().as_str(),
        ]))
        .with_physical_effect_admission(&root_budget)
        .with_progress_report(progress.clone())
        // A headless run waits one request-deadline slice at a time for a
        // background exit instead of ending and polling; the job's wall clock
        // still bounds the whole wait.
        .with_background_wait(std::time::Duration::from_secs(
            config.ai.request_timeout_seconds,
        ))
        .with_mailbox(mailbox)
        // This job's blocks are the continuation snapshot, not database
        // context, so the framing says only that they are untrusted data.
        .with_context_preamble(saya_agent::ContextPreamble::Coding)
        .with_request_preflight(
            crate::agent::preflight_config::request_preflight_config(&config.ai)
                .map_err(|_| CodingError::InvalidRequest)?,
        );
        // The shell route for local data files is decided once, from the
        // checkout this run owns, the `PATH` this process was started with, and
        // whether this root's toolset actually offers `shell`: the route names
        // that tool, so a composition without one must not carry it.
        let file_sources = crate::agent::file_sources::for_session(
            tools.offers_shell(),
            crate::agent::file_sources::detect(
                tools.workspace(),
                std::env::var_os("PATH").as_deref(),
            ),
        );
        let request = AgentRequest {
            prompt: self.request.task.clone(),
            profile_names: vec![],
            model: config.ai.model.clone(),
            system_prompt: Some(super::prompts::system_with_file_sources(
                self.request.plan,
                self.is_read_only_root(),
                file_sources,
            )),
            history: vec![],
            context_blocks: self.continuation_context().await?,
        };
        self.lifecycle().enter("model_loop");
        let outcome = run_agent_with_control(
            provider.as_ref(),
            executor.as_ref(),
            request,
            tools.definitions(),
            control,
        )
        .await;
        self.lifecycle().enter("after_loop");
        // The root loop has finished: record its progress as durable job
        // events, then the final report, then run the `stop` hooks. Reporting
        // is not authority, so its failure never changes the run's outcome.
        let checks = checks
            .lock()
            .map(|checks| checks.clone())
            .unwrap_or_default();
        for check in checks {
            let started = std::time::Instant::now();
            self.record_progress_event(check).await;
            self.lifecycle()
                .note_elapsed("progress_event", started.elapsed());
        }
        if let Some(snapshot) = progress.snapshot() {
            let started = std::time::Instant::now();
            super::progress_report::record(&self.records, &snapshot.report);
            self.lifecycle()
                .note_elapsed("progress_persist", started.elapsed());
        }
        // The root loop has finished: the `stop` hooks run now, after it and
        // whatever its outcome was. They observe the run's end and decide
        // nothing about it.
        if let Some(hooks) = hooks.as_ref() {
            hooks.stop().await;
            // The job root has ended: the end hooks observe it, bounded by
            // their own timeouts and never able to change the outcome.
            hooks.session_end().await;
        }
        // The last turn has no later boundary to be checkpointed at, so the
        // run end takes it, over the checkout this run already resolved.
        let started = std::time::Instant::now();
        self.capture_rewind_at(checkpointed.checkout()).await;
        self.lifecycle()
            .note_elapsed("rewind_checkpoint", started.elapsed());
        match outcome {
            Ok(output) => {
                // A clean progress stop ended the loop `Ok`; the final turn
                // wrote its best state, and the candidate is prepared as for
                // a normal finish.
                if let Some(stop) = progress.snapshot().and_then(|snapshot| snapshot.stop) {
                    self.latch_progress_stop(stop);
                }
                Ok(output)
            }
            Err(error) => {
                // The owner's own stop reaches the loop as a cancellation; the cause is the stop.
                if matches!(error, AgentError::Cancelled)
                    && let Some(failure) = watched.failure()
                {
                    return Err(CodingError::Failed(Box::new(failure)));
                }
                let backstop = self.backstop_failure().await;
                Err(map_root_error(error, config.ai.max_output_tokens, backstop))
            }
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
