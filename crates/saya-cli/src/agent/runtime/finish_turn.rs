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
    // The usage the extraction call reported, if any. Stays `None` when
    // learning is disabled, the gate declined, the breaker has already
    // tripped this session, or the call produced no response — absent is
    // not zero.
    let learning_usage = super::learning::after_turn::learn_after_turn(
        super::learning::after_turn::LearnAfterTurnInputs {
            override_log,
            cancelled: cancellation.is_cancelled(),
            permit_candidate_writes,
            database,
            output: output.as_ref().ok(),
            prompt,
            observations_log,
            receipt,
            provider,
            model,
            // The breaker is session-scoped so it trips across this session's
            // turns; no session (the one-shot `ask` path, each candidate
            // attempt) means a fresh one this call, which can never trip
            // within one turn, since there is no next turn for it to disable.
            breaker: session.map(|session| session.learning_breaker()),
        },
        sink,
    )
    .await;

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
        // Agent-layer failures: the effect system refused or failed on our
        // side, not the provider. `Provider` is reserved for real provider
        // failures because it feeds the provider-down exit code.
        AgentError::EffectAdmissionDenied
        | AgentError::UnsupportedPhysicalCoverage
        | AgentError::EffectOwnerFailure(_) => {
            AgentRuntimeError::Agent(error.to_string())
        }
        AgentError::Cancelled => AgentRuntimeError::Agent("request cancelled".into()),
        AgentError::Progress(_) => AgentRuntimeError::Agent(error.to_string()),
        // A future agent-layer error is still a failure; carry its own text so
        // the operator sees the real cause rather than a silent success.
        _ => AgentRuntimeError::Agent(error.to_string()),
    })
}
