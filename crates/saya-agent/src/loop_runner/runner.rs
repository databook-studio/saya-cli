use crate::{
    ActorCorrelationDigest, AgentEventSink, AgentMetric, AgentMetricOutcome, AgentMetricPhase,
    AgentRequest, ApprovalDecider, CancellationToken, ChatMessage, ChatProvider, CorrelationId,
    EffectAdmission, TokenUsage, ToolDefinition, ToolExecutor,
};

use super::{
    AgentError, AgentLimits, AgentOutput, PartialOutput, ProgressHandle, ProgressStop,
    ProgressStopKind, admission, budget_output, control, progress::ProgressClassifier, salvage,
    turn,
};

pub(super) mod background;
mod note;

use note::refresh_turn_note;

pub(super) async fn run_agent_inner(
    invocation: control::AgentRunInvocation<'_>,
) -> Result<AgentOutput, AgentError> {
    let conversation_correlation = invocation.answering.as_ref().map_or_else(
        CorrelationId::next,
        control::AnsweringScope::conversation_correlation,
    );
    let invocation_correlation = CorrelationId::next();
    if let Some(scope) = invocation.answering.clone() {
        let mut admission = scope.lock().await?;
        admission.begin_invocation();
        run_agent_loop(
            invocation,
            &mut admission,
            conversation_correlation,
            invocation_correlation,
        )
        .await
    } else {
        let mut admission = admission::Admission::new(super::AnsweringLimits::default(), false);
        run_agent_loop(
            invocation,
            &mut admission,
            conversation_correlation,
            invocation_correlation,
        )
        .await
    }
}

async fn run_agent_loop(
    invocation: control::AgentRunInvocation<'_>,
    admission: &mut admission::Admission,
    conversation_correlation: CorrelationId,
    invocation_correlation: CorrelationId,
) -> Result<AgentOutput, AgentError> {
    let control::AgentRunInvocation {
        mut mailbox,
        provider,
        tools,
        request,
        definitions,
        limits,
        approval,
        sink,
        cancellation,
        answering,
        request_preflight,
        effect_admission,
        require_physical_effect_admission,
        actor_correlation,
        authority_refusal_context,
        context_preamble,
        progress_report,
        background_wait,
    } = invocation;
    let controlled = answering.is_some();
    admission.initialize(sink).await;
    if controlled {
        admission.check(&cancellation)?;
    }
    let assembly_started = std::time::Instant::now();
    let assembled = crate::history::build_messages_with_history_count(
        request.system_prompt.as_deref(),
        &request.context_blocks,
        &request.prompt,
        &request.history,
        limits.context_byte_budget,
        context_preamble,
    );
    sink.metric(AgentMetric::Phase {
        phase: AgentMetricPhase::MessageAssembly,
        elapsed_ns: super::attempt_metrics::duration_ns(assembly_started.elapsed()),
        outcome: if assembled.is_ok() {
            AgentMetricOutcome::Succeeded
        } else {
            AgentMetricOutcome::Failed
        },
    })
    .await;
    let (messages, ordinary_history_pairs, protected_prefix_len) = assembled?;
    let progress = limits.progress.map(ProgressClassifier::new);
    let mut state = RunState {
        progress,
        progress_report,
        background_wait,
        final_turn: None,
        last_assistant_text: String::new(),
        provider,
        tools,
        request,
        definitions,
        limits,
        approval,
        sink,
        cancellation,
        request_preflight,
        effect_admission,
        require_physical_effect_admission,
        conversation_correlation,
        invocation_correlation,
        actor_correlation,
        request_ordinal: 0,
        assistant_replies_in_run: false,
        consecutive_stream_cuts: 0,
        previous_diagnostic_failure: false,
        messages,
        checkpoint: super::coding_checkpoint::CodingCheckpoint::with_authority(
            authority_refusal_context,
        ),
        ordinary_history_pairs,
        protected_prefix_len,
        events: Vec::new(),
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        continuation_count: 0,
        pending_continuation: None,
        // Calls that failed during this run — SQL statements keyed on the
        // statement exactly as submitted, other tools on (tool, arguments) — so a
        // byte-identical re-submission is refused rather than re-executed (loop
        // invariant for the "do not repeat a failed call" advice the model does
        // not always obey).
        failed: super::failed_statements::FailedStatements::new(),
        // The last statement that completed successfully, so a run that exhausts its
        // budget without nominating can still surface its best available answer.
        last_successful_sql: None,
        designation: super::designation::State::new(),
    };
    loop {
        if let Some(mailbox) = mailbox.as_mut() {
            mailbox
                .boundary(&state.cancellation, &mut state.messages, state.tools)
                .await?;
        }
        if let Err(error) = admission.check(&state.cancellation) {
            if let Some(dimension) = admission.stopped_dimension(&error, &state.cancellation) {
                return Ok(budget_output(
                    dimension,
                    &mut state.events,
                    state.sink,
                    PartialOutput {
                        usage: state.usage,
                        used_bounded_sql_query: state.used_bounded_sql_query,
                        tool_metadata: state.tool_metadata,
                        answer_sql: state.designation.sql.clone().or(state.last_successful_sql),
                        answer: String::new(),
                    },
                )
                .await);
            }
            return Err(error);
        }
        if admission.legacy.turn_limit(state.limits.max_turns) {
            let first_request = state.first_request();
            return salvage::salvage(
                state.provider,
                &state.request.model,
                &mut state.messages,
                &mut state.ordinary_history_pairs,
                &mut state.protected_prefix_len,
                &[],
                &state.checkpoint,
                state.limits.context_byte_budget,
                state.request_preflight,
                state.sink,
                &state.cancellation,
                &mut state.events,
                &mut state.usage,
                admission,
                state.used_bounded_sql_query,
                state.tool_metadata,
                state.designation.sql.clone().or(state.last_successful_sql),
                state.effect_admission,
                state.require_physical_effect_admission,
                state.conversation_correlation,
                state.invocation_correlation,
                state.actor_correlation,
                state.request_ordinal.saturating_add(1),
                first_request,
            )
            .await;
        }
        // A background run that ended since the last request is told to the
        // model here, after the turn's tool results and before the next
        // request; several exits travel as one message.
        if let Some(notice) = background::notice(state.tools) {
            // An attended surface shows the same line; other sinks drop it.
            state.sink.background_exit(&notice).await;
            state.messages.push(ChatMessage::text("user", notice));
        }
        refresh_turn_note(&mut state.messages, state.tools.turn_context_note());
        // A surface whose list can grow re-states it here: the coding root
        // advertises the two calls that reach a background run only once this
        // job has started one, so the list the run began with is not the list
        // every later request carries.
        if state.final_turn.is_none()
            && let Some(definitions) = state.tools.advertised_definitions()
        {
            state.definitions = definitions;
        }
        if let Some(output) = turn::run(&mut state, admission).await? {
            // The answering turn is a model turn too, so the report counts it;
            // any guard it would trip is ignored because the run is ending.
            state.finish_progress_turn();
            // A final turn that answers returns `Ok`; the stop that started
            // the wind-down must still travel to the caller.
            let stop = state
                .final_turn
                .as_ref()
                .map(|final_turn| final_turn.stop.clone());
            state.publish_progress(stop);
            return Ok(output);
        }
        // The final turn after a progress stop is bounded: the model gets one
        // chance to update the checklist and one to write its summary, and
        // then the job ends with whatever it produced.
        if let Some(final_turn) = state.final_turn.as_mut() {
            final_turn.turns_left = final_turn.turns_left.saturating_sub(1);
            if final_turn.turns_left != 0 {
                continue;
            }
            let stop = final_turn.stop.clone();
            // This wind-down turn ran; count it before the forced end.
            state.finish_progress_turn();
            state.publish_progress(Some(stop));
            return Ok(state.final_output());
        }
        if let Some(stop) = state.take_pending_stop() {
            state.begin_final_turn(stop);
            continue;
        }
        if let Some(stop) = state.finish_progress_turn() {
            if stop.kind.is_progress() {
                state.begin_final_turn(stop);
                continue;
            }
            return Err(AgentError::Progress(Box::new(stop)));
        }
    }
}

/// How many model turns the clean wind-down gets: one to update the
/// checklist and one to write the summary. Bounded, so a model that ignores
/// the instruction cannot keep the job alive.
const FINAL_TURN_TURNS: u32 = 2;

pub(super) struct FinalTurn {
    stop: ProgressStop,
    turns_left: u32,
}

pub(super) struct RunState<'a> {
    pub(super) progress: Option<ProgressClassifier>,
    pub(super) progress_report: Option<ProgressHandle>,
    /// The per-slice wait for a background exit on a headless run. `None`
    /// ends a no-tool-call turn as it always did.
    pub(super) background_wait: Option<std::time::Duration>,
    pub(super) final_turn: Option<FinalTurn>,
    /// The last assistant text, kept so a forced wind-down can still return
    /// something the model wrote.
    pub(super) last_assistant_text: String,
    pub(super) provider: &'a dyn ChatProvider,
    pub(super) tools: &'a dyn ToolExecutor,
    pub(super) request: AgentRequest,
    pub(super) definitions: Vec<ToolDefinition>,
    pub(super) limits: AgentLimits,
    pub(super) approval: &'a dyn ApprovalDecider,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: CancellationToken,
    pub(super) request_preflight: Option<super::RequestPreflightConfig>,
    pub(super) effect_admission: Option<&'a dyn EffectAdmission>,
    pub(super) require_physical_effect_admission: bool,
    pub(super) conversation_correlation: CorrelationId,
    pub(super) invocation_correlation: CorrelationId,
    pub(super) actor_correlation: Option<ActorCorrelationDigest>,
    pub(super) request_ordinal: u64,
    /// An assistant reply has been committed to `messages` in this run. The
    /// first request of a run (and a continuation after a discarded reply)
    /// has none, which is the adaptive policy's "think hard" arm.
    pub(super) assistant_replies_in_run: bool,
    /// Cuts since the last completed reply. A cut answers with its own
    /// continuation allowance, and a reply resets the count so cuts spread
    /// over a long run never exhaust it.
    pub(super) consecutive_stream_cuts: u32,
    /// The trailing results of the previous request contained a diagnostic
    /// failure. A second consecutive one thinks medium instead of high.
    pub(super) previous_diagnostic_failure: bool,
    pub(super) messages: Vec<ChatMessage>,
    pub(super) checkpoint: super::coding_checkpoint::CodingCheckpoint,
    pub(super) ordinary_history_pairs: usize,
    pub(super) protected_prefix_len: usize,
    pub(super) events: Vec<crate::AgentEvent>,
    pub(super) used_bounded_sql_query: bool,
    pub(super) tool_metadata: Vec<crate::ToolMetadata>,
    pub(super) usage: TokenUsage,
    pub(super) continuation_count: u32,
    pub(super) pending_continuation: Option<super::receive::ContinuationProgress>,
    pub(super) failed: super::failed_statements::FailedStatements,
    pub(super) last_successful_sql: Option<String>,
    pub(super) designation: super::designation::State,
}

impl RunState<'_> {
    /// The adaptive policy's "think hard" arm: no assistant reply has been
    /// committed, and no cut has already discarded one. A request after a cut
    /// is not the run's first request — the model already planned once.
    pub(super) fn first_request(&self) -> bool {
        !self.assistant_replies_in_run && self.consecutive_stream_cuts == 0
    }

    /// Publishes the classifier's current view to the caller's handle.
    fn publish_progress(&self, stop: Option<ProgressStop>) {
        if let (Some(classifier), Some(handle)) = (&self.progress, &self.progress_report) {
            handle.publish(classifier.report(), stop);
        }
    }

    /// Ends one model turn for the progress guards and publishes the result.
    fn finish_progress_turn(&mut self) -> Option<ProgressStop> {
        let cost = self.usage.cost_micros;
        let stop = self
            .progress
            .as_mut()
            .and_then(|classifier| classifier.finish_turn(cost));
        self.publish_progress(stop.clone());
        stop
    }

    /// A repeat stop detected while the turn's tool batch settled.
    fn take_pending_stop(&mut self) -> Option<ProgressStop> {
        let stop = self
            .progress
            .as_mut()
            .and_then(ProgressClassifier::take_stop);
        if let Some(stop) = &stop {
            self.publish_progress(Some(stop.clone()));
        }
        stop
    }

    /// Winds the job down cleanly: one bounded final phase in which only the
    /// checklist is available, then the candidate is prepared as usual.
    fn begin_final_turn(&mut self, stop: ProgressStop) {
        self.final_turn = Some(FinalTurn {
            stop: stop.clone(),
            turns_left: FINAL_TURN_TURNS,
        });
        self.definitions
            .retain(|definition| definition.name == "checklist");
        self.messages
            .push(ChatMessage::text("user", final_turn_note(&stop)));
    }

    /// The output a forced wind-down returns when the model did not produce a
    /// final answer within its bounded turns.
    fn final_output(&mut self) -> AgentOutput {
        AgentOutput {
            answer: std::mem::take(&mut self.last_assistant_text),
            events: std::mem::take(&mut self.events),
            used_bounded_sql_query: self.used_bounded_sql_query,
            tool_metadata: std::mem::take(&mut self.tool_metadata),
            usage: self.usage,
            learning_usage: None,
            truncated: false,
            answer_sql: self.designation.sql.clone(),
        }
    }
}

/// The model-facing instruction for the clean wind-down. It names the guard
/// that fired in plain terms and the one tool that remains.
fn final_turn_note(stop: &ProgressStop) -> String {
    let reason = match stop.kind {
        ProgressStopKind::NoProgress => "it stopped making progress",
        ProgressStopKind::Repeating => "a tool call kept returning the same result",
        ProgressStopKind::WallClock | ProgressStopKind::Cost => "a limit was reached",
    };
    format!(
        "The job is stopping because {reason}. Only the checklist tool is available now. \
         Write your best current state and a short summary now."
    )
}
