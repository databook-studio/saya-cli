// Each input is consumed by a distinct post-turn action; keeping the handoff
// explicit makes the learning and cancellation dependencies visible.
#[allow(clippy::too_many_arguments)]
async fn finish_turn(
    mut output: Result<AgentOutput, AgentError>,
    override_log: Arc<tools::OverrideLog>,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    session: Option<&Arc<SessionUniverse>>,
    permit_candidate_writes: bool,
    database: &Arc<tools::DatabaseTools>,
    prompt: &str,
    observations_log: Option<&tools::ObservationLog>,
    receipt: &Arc<crate::contracts::RecallReceipt>,
    provider: &dyn saya_agent::ChatProvider,
    model: &str,
) -> Result<AgentOutput, AgentRuntimeError> {
    let overridden = override_log.drain();
    if !overridden.is_empty() {
        sink.emit(AgentEvent::knowledge_overridden(overridden.clone()))
            .await;
    }
    // The breaker: session-scoped so it trips across this session's turns; a
    // fresh one per call when there is no session (the one-shot `ask` path,
    // each candidate attempt) — which can never trip within one turn, since
    // there is no next turn for it to disable.
    let fresh_breaker;
    let breaker: &super::learning::LearningBreaker = match session {
        Some(session) => session.learning_breaker(),
        None => {
            fresh_breaker = super::learning::LearningBreaker::new();
            &fresh_breaker
        }
    };
    // The usage the extraction call reported, if any. Stays `None` when
    // learning is disabled, the gate declined, the breaker has already
    // tripped this session, or the call produced no response — absent is
    // not zero.
    let learning_usage = if cancellation.is_cancelled() {
        None
    } else {
        super::learning::post_turn::run_post_turn_extraction(
            super::learning::post_turn::PostTurnInputs {
                permit_candidate_writes,
                database,
                output: output.as_ref(),
                prompt,
                observations_log,
                receipt,
                overridden: &overridden,
                provider,
                model,
                breaker,
            },
            sink,
        )
        .await
    };

    // Attach the extraction usage to the output so both the TUI and headless
    // recorders can fold it into the learning total. A failed answering call
    // leaves the output as `Err`, so the learning usage is simply dropped.
    if let Ok(out) = output.as_mut() {
        out.learning_usage = learning_usage;
    }

    output.map_err(|error| match error {
        AgentError::Provider(error) => AgentRuntimeError::Provider(provider_message(&error)),
        AgentError::Limit(error) => {
            AgentRuntimeError::Agent(format!("agent limit reached: {error}"))
        }
        AgentError::InvalidToolCall => {
            AgentRuntimeError::Agent("provider returned an unsupported tool call".into())
        }
        AgentError::InvalidHistory => {
            AgentRuntimeError::Agent("conversation history is invalid".into())
        }
        // The harness classifies these agent-layer failures as provider
        // failures (see `failure_code` in saya-harness); the user-visible
        // message is the error's own display string either way, and the turn
        // fails rather than succeeding.
        AgentError::EffectAdmissionDenied
        | AgentError::UnsupportedPhysicalCoverage
        | AgentError::EffectOwnerFailure(_) => {
            AgentRuntimeError::Provider(error.to_string())
        }
        AgentError::Cancelled => AgentRuntimeError::Agent("request cancelled".into()),
    })
}
