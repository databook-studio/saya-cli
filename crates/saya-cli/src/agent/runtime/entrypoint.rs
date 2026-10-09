/// Production entry point: builds provider + registry from config and executes the turn.
///
/// `can_prompt` means "this surface may read stdin" — it feeds the
/// connector's secret prompt, the turn's fallback [`TerminalApproval`], and
/// nothing else. `can_obtain_approval` means "this surface can obtain a
/// per-call approval at all" — it feeds the advertisement gate
/// ([`SessionUniverse::definitions`] for sessions, the one-shot branch
/// below for `ask`), never the stdin fallback. The line REPL passes its
/// live-terminal fact for both; the TUI passes `false` for the first and
/// `true` for the second (its modal); the one-shot `ask` path passes the
/// same value for both (unchanged behaviour). Keep them split. The
/// session's grant view rides the universe the session surfaces bind per
/// turn (`session_grant_surface`): a held `workspace-write` grant widens
/// only that family under `ask`, surface or no surface.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_prompt_with_sink(
    runtime: &RuntimeConfig,
    prompt: &str,
    approval: ApprovalPolicy,
    can_prompt: bool,
    can_obtain_approval: bool,
    overrides: PromptOverrides,
    history: Vec<ChatMessage>,
    sink: &dyn AgentEventSink,
    cancellation: CancellationToken,
    question_scope: Option<QuestionScope>,
    state_db: Option<SqliteStateStore>,
    decider: Option<Arc<dyn ApprovalDecider>>,
    last_sql: Option<String>,
    // The session's tool universe, composed once per interactive session.
    // `None` (the one-shot `ask` path) leaves `workspace_read` denying with
    // a typed error and the write-shaped tools hidden.
    session: Option<Arc<SessionUniverse>>,
    // The agent's task posture, threaded like `approval`: the session's
    // `/mode` state at the composition root.
    agent_mode: AgentMode,
    // The TUI's capture hook, `Some` only where a stream channel exists; the
    // headless paths pass `None` and the tools never capture (C1/C2).
    capture: Option<tools::CaptureHook>,
) -> Result<AgentOutput, AgentRuntimeError> {
    run_prompt_with_sink_and_outcomes(SinkPrompt {
        runtime,
        prompt,
        approval,
        can_prompt,
        can_obtain_approval,
        overrides,
        history,
        sink,
        cancellation,
        question_scope,
        state_db,
        decider,
        last_sql,
        session,
        agent_mode,
        capture,
        prior_tool_outcomes: None,
        continuation: None,
        compaction_summary: None,
    })
    .await
}
