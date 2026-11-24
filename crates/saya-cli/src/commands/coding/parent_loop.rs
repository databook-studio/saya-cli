//! One root turn: the loop over the run's composition, and the work that
//! follows it — the durable progress events, the `stop` and `session_end`
//! hooks, the final checkpoint and the failure mapping.
//!
//! A turn builds only what belongs to a turn: the sink chain over the
//! downstream sink, the run control, the progress handle and the request. The
//! provider, toolset, hooks and checkout come from the run's one-time setup
//! (`RootSession`), so a second turn costs no composition.

use super::parent_arming::{TurnArming, stated_delay, stop_the_turn};
use super::parent_turn::RootSession;
use super::{CodingError, NativeCodingJob, map_root_error};
use crate::budget_wording::{budget_dimension, preflight_dimension};
use crate::commands::coding::TurnStop;
use saya_agent::{
    AgentError, AgentEventSink, AgentOutput, AgentRequest, ApprovalDecider, ToolExecutor,
    run_agent_with_control,
};
use std::sync::Arc;

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
        // This turn's stop signal, a child of the run's: a stop of the turn —
        // an attached surface's Esc, a surface's cancel — ends this turn and
        // leaves the run taking turns, while a stop of the run ends the turn it
        // is running.
        let scope = self.root_run.begin_turn(&self.cancellation);
        let cancellation = scope.token();
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
        let (progress, checks) =
            super::parent_progress::handle(&self.records, self.lifecycle_arc());
        // A `PermissionRequest` hook can only refuse an ask this root would
        // make, so the gate wraps the toolset where the hooks are composed.
        let approver: &dyn ApprovalDecider = match &session.approval_gate {
            Some(gate) => gate.as_ref(),
            None => session.tools.as_ref(),
        };
        let root_budget = self.ledger.root_view();
        // This turn's plugin budget: unarmed until the turn first uses a plugin.
        let arming = TurnArming::new(session.plugins.clone(), cancellation.clone());
        let turn_control = super::parent_control::build(
            self,
            session,
            approver,
            &checkpointed,
            cancellation.clone(),
            &progress,
            &root_budget,
        )?;
        let control = turn_control.control.with_deferred_budget(arming.scope());
        let turn_budget = turn_control.budget;
        let deadline = turn_control.deadline;
        // The prompt as it was sent: the prompt half of the pair a later turn
        // of this run replays.
        let asked = prompt.clone();
        // This turn's context comes first: the root shell's composition proves
        // the sandbox by creating and removing canary directories inside W, and
        // a readiness walk that met one of them mid-removal would send the model
        // a different view than the run's first request must carry. A one-turn
        // composition inherits nothing, so it states none of it.
        let mut context_blocks = if self.application.inherits_continuation_context() {
            self.continuation_context().await?
        } else {
            Vec::new()
        };
        // A chat surface carries the project's own untrusted context too — the
        // instruction sources and the worktree's explicit preferences, on the
        // same labelled lane and through the one injection site the interactive
        // turn uses. It is stable across the run's turns, so it sits ahead of
        // the continuation blocks and keeps the request prefix the provider
        // caches.
        if session.project_context {
            context_blocks = crate::agent::context::projection::project_untrusted_context(
                &self.application.runtime,
                Some(self.application.root_path()),
                Some(&self.application.store),
                &asked,
                session.prompt_for_turn().as_deref(),
                context_blocks,
            )
            .await;
        }
        // The workspace summary the launch gathered in the background rides the
        // request's untrusted lane, after the sources every turn repeats: the
        // instructions and the worktree's preferences are the request's stable
        // prefix, and a fact the model needs once belongs behind them rather
        // than ahead of them. It is carried by the run's first turn only, and
        // never waited for: a scan still running when the message was sent adds
        // nothing.
        if let Some(block) = session.summary_block(turn) {
            context_blocks.push(block);
        }
        // The session's own facts join it: the task list the run holds, the
        // compaction narrative and the continuation record its loop holds now,
        // each through the builder the interactive turn already renders them
        // with. A run no session stated them for (every job surface) carries
        // none of them.
        if let Some(block) = session.tools.take_session_tasks() {
            context_blocks.push(block);
        }
        if let Some(facts) = session.facts.as_ref() {
            context_blocks.extend(facts.blocks(config.ai.context_byte_budget));
        }
        // The active plugins' context joins it, bounded by the same byte
        // budget; plugin context in this turn's prompt is a use of the plugin,
        // so its policy arms here, before request 1.
        if let Some(plugins) = session.plugins.as_ref() {
            let blocks =
                super::parent_plugins::context(plugins, &asked, config.ai.context_byte_budget)
                    .await;
            arming.arm_on_context(&blocks).await;
            context_blocks.extend(blocks);
            // What the plugin's recall supplied is stated here — after the
            // context is assembled and before the first request is sent, the
            // order the old turn body stated it in. One event per turn, on the
            // same sink the turn's other events ride.
            if let Some(event) = plugins.knowledge_event() {
                checkpointed.emit(event).await;
            }
        }
        session.tools.start_shell_composition();
        // The text this turn asks with is the run's own request, and the
        // optional tools are advertised from it: a job root's first turn is its
        // task, a chat session's is the message the operator sent. Stated once,
        // so the advertisement — and the request prefix a provider caches — is
        // stable across the run's turns.
        session.tools.note_request_text(&asked);
        let request = AgentRequest {
            prompt,
            profile_names: vec![],
            model: session.model.clone(),
            system_prompt: session.prompt_for_turn(),
            // Chat's own shape of history — one (prompt, final answer) pair per
            // completed turn, tool arguments and results never replayed — left
            // for the shared assembly to bound by this run's context byte
            // budget, exactly as chat's is bounded.
            history: session.history.replay(),
            context_blocks,
        };
        self.lifecycle().enter("model_loop");
        // A name an active plugin declares is the plugin's to serve; every
        // other name is the run's own executor. With no plugins the run hands
        // the loop exactly the executor it always did.
        let routed = session.plugins.as_ref().map(|plugins| {
            super::parent_plugins::Routed::new(
                Arc::clone(plugins),
                Arc::clone(&session.executor),
                arming.clone(),
                self.tools().clone(),
            )
        });
        let executor: &dyn ToolExecutor = match &routed {
            Some(routed) => routed,
            None => session.executor.as_ref(),
        };
        // The core tools first, then the active plugins', as a helper composes
        // them; with no plugins this is the list the core advertises.
        let definitions = match session.plugins.as_ref() {
            Some(plugins) => super::parent_plugins::definitions(
                session.tools.definitions(),
                plugins,
                self.tools(),
            ),
            None => session.tools.definitions(),
        };
        let work = run_agent_with_control(
            session.provider.as_ref(),
            executor,
            request,
            definitions,
            control,
        );
        tokio::pin!(work);
        // Two elapsed owners, one wait: the deadline a stated turn policy names,
        // and the plugin budget's own, whose sleep starts at its arm.
        let outcome = tokio::select! {
            biased;
            outcome = &mut work => outcome,
            () = arming.expire() => {
                stop_the_turn(&cancellation);
                work.await
            }
            () = stated_delay(deadline) => {
                if let Some(scope) = turn_budget.as_ref() {
                    stop_the_turn(&scope.cancellation_token());
                }
                work.await
            }
        };
        let mut outcome = outcome;
        self.lifecycle().enter("after_loop");
        // The turn's end is stated to the active plugins once, whatever it was:
        // a plugin observes the outcome and never changes it, and what the
        // plugin's own post-turn work spent is folded into the turn's report —
        // apart from the answering total, so `/usage` can label the two calls.
        if let Some(plugins) = session.plugins.as_ref() {
            let finished = saya_agent::TurnOutcome {
                answered: outcome.is_ok(),
                cancelled: matches!(&outcome, Err(AgentError::Cancelled)),
                prompt: &asked,
                output: outcome.as_ref().ok(),
            };
            let learning_usage =
                super::parent_plugins::turn_finished(plugins, &finished, sink).await;
            if let (Some(usage), Ok(output)) = (learning_usage, &mut outcome) {
                output.learning_usage = Some(usage);
            }
        }
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
                // The question budget stopped THIS turn, not the run: chat's
                // partial wording says so, and the next turn is admitted afresh.
                if let Some(dimension) = turn_budget
                    .as_ref()
                    .and_then(|scope| budget_dimension(scope.cancellation_token().reason()))
                    .or_else(|| arming.stop_dimension())
                    // A turn the engine refused before admission stated its own
                    // stop on its token and handed back partial output: no
                    // plugin budget armed and the question's scope spent
                    // nothing, so this is the only reader of that stop. It is
                    // still the budget stop, never an answer.
                    .or_else(|| preflight_dimension(cancellation.reason()))
                {
                    self.root_run
                        .note_turn_stop(Some(TurnStop::budget(dimension)));
                } else if let Some(stop) = progress.snapshot().and_then(|snapshot| snapshot.stop) {
                    // A clean progress stop ended the loop `Ok`; the final turn
                    // wrote its best state, and the candidate is prepared as for
                    // a normal finish. The stop reads as a pause, with the
                    // guard's own class and message.
                    let failure = self.latch_progress_stop(stop);
                    self.root_run
                        .note_turn_stop(Some(TurnStop::of_failure(&failure)));
                } else {
                    self.root_run.note_turn_stop(None);
                }
                Ok(output)
            }
            Err(error) => {
                // The owner's own stop reaches the loop as a cancellation; the cause is the stop.
                if matches!(error, AgentError::Cancelled)
                    && let Some(failure) = watched.failure()
                {
                    self.root_run
                        .note_turn_stop(Some(TurnStop::of_failure(&failure)));
                    return Err(CodingError::Failed(Box::new(failure)));
                }
                let backstop = self.backstop_failure().await;
                let error = map_root_error(error, config.ai.max_output_tokens, backstop);
                self.root_run.note_turn_stop(match &error {
                    CodingError::Failed(failure) => Some(TurnStop::of_failure(failure)),
                    _ => None,
                });
                Err(error)
            }
        }
    }
}
