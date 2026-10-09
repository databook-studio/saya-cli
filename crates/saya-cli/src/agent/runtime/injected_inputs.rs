/// The turn body, injectable for tests via [`TurnInputs`].
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_prompt_with_inputs(
    runtime: &RuntimeConfig,
    inputs: TurnInputs,
    prompt: &str,
    approval: ApprovalPolicy,
    can_prompt: bool,
    can_obtain_approval: bool,
    history: Vec<ChatMessage>,
    sink: &dyn AgentEventSink,
    cancellation: CancellationToken,
    state_db: Option<SqliteStateStore>,
    decider: Option<Arc<dyn ApprovalDecider>>,
    last_sql: Option<String>,
    // The session's tool universe, composed once per interactive session.
    // `None` (the one-shot `ask` path) leaves `workspace_read` denying with
    // a typed error and the write-shaped tools hidden.
    session: Option<Arc<SessionUniverse>>,
    // The agent's task posture, threaded like `approval`.
    agent_mode: AgentMode,
    // The TUI's capture hook (C1); `None` for every headless caller.
    capture: Option<tools::CaptureHook>,
    // The session's saved continuation record, carried like the live
    // outcome bundle: `Some` only where a session state exists.
    continuation: Option<saya_store::StoredContinuation>,
) -> Result<AgentOutput, AgentRuntimeError> {
    run_prompt_with_continuation_inputs_with_signal(
        TurnPrompt {
            runtime,
            inputs,
            prompt,
            approval,
            can_prompt,
            can_obtain_approval,
            history,
            sink,
            cancellation,
            question_scope: None,
            state_db,
            decider,
            last_sql,
            session,
            agent_mode,
            capture,
            prior_tool_outcomes: None,
            continuation,
            compaction_summary: None,
        },
        None,
    )
    .await
}
