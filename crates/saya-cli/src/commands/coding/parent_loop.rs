//! One root turn: the loop over the run's composition, and the work that
//! follows it — the durable progress events, the `stop` and `session_end`
//! hooks, the final checkpoint and the failure mapping.
//!
//! A turn builds only what belongs to a turn: the sink chain over the
//! downstream sink, the run control, the progress handle and the request. The
//! provider, toolset, hooks and checkout come from the run's one-time setup
//! (`RootSession`), so a second turn costs no composition.

use super::parent_turn::RootSession;
use super::{CodingError, NativeCodingJob, map_root_error};
use saya_agent::{
    AgentError, AgentEventSink, AgentOutput, AgentRequest, AgentRunControl, ApprovalDecider,
    ContextPreamble, ProgressCheck, ProgressHandle, run_agent_with_control,
};
use std::sync::{Arc, Mutex};

impl NativeCodingJob {
    /// Runs one root turn on the run's composition and returns its output.
    pub(super) async fn run_turn(
        &self,
        session: &RootSession,
        sink: &dyn AgentEventSink,
        prompt: String,
    ) -> Result<AgentOutput, CodingError> {
        // Every turn states its own boundary before it asks the model anything:
        // the run's epoch and this turn's fresh sequence, so a message after the
        // first turn is as durable a record as the turn that opened the run.
        let turn = self.root_run.next_turn();
        self.record_turn_boundary(turn).await?;
        let config = &self.application.runtime.resolved;
        let observed = super::super::observations::NativeSink {
            job: self,
            downstream: sink,
        };
        let watched = super::super::convergence::ConvergenceSink::new(self, &observed);
        // The turn-boundary checkpoint rides outside the stall watcher: it
        // observes the same events and changes nothing about them.
        let checkpointed =
            super::super::rewind::RewindSink::new(self, &watched, session.checkout.clone());
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
            super::super::progress_report::record(&progress_records, &snapshot.report);
            progress_lifecycle.note_elapsed("progress_persist", started.elapsed());
        }));
        // A `PermissionRequest` hook can only refuse an ask this root would
        // make, so the gate wraps the toolset where the hooks are composed.
        let approver: &dyn ApprovalDecider = match &session.approval_gate {
            Some(gate) => gate.as_ref(),
            None => session.tools.as_ref(),
        };
        let root_budget = self.ledger.root_view();
        let control = AgentRunControl::new(
            super::super::limits::agent_limits(config),
            super::super::limits::answering_limits(config),
            approver,
            &checkpointed,
            self.cancellation.clone(),
        )
        .with_actor_correlation(super::super::create::actor_correlation_digest(&[
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
        ));
        // The run's mailbox belongs to its first turn: the loop consumes the
        // receiver, so only that turn can carry it.
        let mailbox = session
            .mailbox
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let mut control = control
            // This job's blocks are the continuation snapshot, not database
            // context, so the framing says only that they are untrusted data.
            .with_context_preamble(ContextPreamble::Coding);
        if let Some(mailbox) = mailbox {
            control = control.with_mailbox(mailbox);
        }
        let control = control.with_request_preflight(
            crate::agent::preflight_config::request_preflight_config(&config.ai)
                .map_err(|_| CodingError::InvalidRequest)?,
        );
        // The prompt as it was sent: the prompt half of the pair a later turn
        // of this run replays.
        let asked = prompt.clone();
        // This turn's context comes first: the root shell's composition proves
        // the sandbox by creating and removing canary directories inside W, and
        // a readiness walk that met one of them mid-removal would send the model
        // a different view than the run's first request must carry.
        let context_blocks = self.continuation_context().await?;
        session.tools.start_shell_composition();
        let request = AgentRequest {
            prompt,
            profile_names: vec![],
            model: session.model.clone(),
            system_prompt: session.system_prompt.clone(),
            // Chat's own shape of history — one (prompt, final answer) pair per
            // completed turn, tool arguments and results never replayed — left
            // for the shared assembly to bound by this run's context byte
            // budget, exactly as chat's is bounded.
            history: session.history.replay(),
            context_blocks,
        };
        self.lifecycle().enter("model_loop");
        let outcome = run_agent_with_control(
            session.provider.as_ref(),
            session.executor.as_ref(),
            request,
            session.tools.definitions(),
            control,
        )
        .await;
        self.lifecycle().enter("after_loop");
        // The root loop has finished: record its progress as durable job
        // events, then the final report, then run the `stop` hooks. Reporting
        // is not authority, so its failure never changes the turn's outcome.
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
            super::super::progress_report::record(&self.records, &snapshot.report);
            self.lifecycle()
                .note_elapsed("progress_persist", started.elapsed());
        }
        // The root loop has finished: the `stop` hooks run now, after it and
        // whatever its outcome was. They observe the run's end and decide
        // nothing about it.
        if let Some(hooks) = session.hooks.as_ref() {
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
                // The run keeps this turn for the next one to replay: the
                // prompt it answered and the answer it settled on, partial
                // included. A failed or cancelled turn is recorded by
                // neither chat nor this run.
                session.history.record(&asked, &output.answer);
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
}
