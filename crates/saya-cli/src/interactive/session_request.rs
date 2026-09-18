use crate::{
    agent::{
        self,
        runtime::{AgentRuntimeError, PromptOverrides},
    },
    config::runtime::RuntimeConfig,
    render::RenderFormat,
    stream_render::TerminalSink,
};
use saya_agent::{
    AgentMode, AgentOutput, ApprovalDecider, ApprovalPolicy, CancellationToken, ChatMessage,
    SessionPolicy,
};
use saya_store::SqliteStateStore;
use std::sync::Arc;

use super::session_universe::SessionUniverse;

pub(crate) enum PromptResult {
    /// Boxed because the variant dwarfs `Cancelled`, which carries nothing;
    /// an unboxed `AgentOutput` makes every `PromptResult` as large as a
    /// completed turn.
    Completed(Box<AgentOutput>),
    Cancelled,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    runtime: &RuntimeConfig,
    prompt: &str,
    approval: ApprovalPolicy,
    // The session's one approval policy, cloned into this turn's decider: a
    // grant recorded in this turn's ask is in force for every later turn.
    policy: SessionPolicy,
    // The session's journal: a `[s]` answer's new grant is journalled there
    // before the call it allowed runs.
    journal: Option<Arc<saya_store::SessionJournal>>,
    can_prompt: bool,
    overrides: PromptOverrides,
    history: Vec<ChatMessage>,
    format: RenderFormat,
    state_db: &SqliteStateStore,
    session: Arc<SessionUniverse>,
    // The agent's task posture, threaded like `approval`: the real source
    // arrives with `/mode` in the next slice.
    agent_mode: AgentMode,
) -> Result<PromptResult, AgentRuntimeError> {
    let cancellation = CancellationToken::new();
    let sink = TerminalSink::new(format);
    // The decider is the terminal ask over the session's policy: the mode
    // resolves there, and a session grant recorded by an answer lands in the
    // one store every turn shares. It holds the session universe's primary
    // handle, which the turn binds from its registry — a grant suggestion
    // names the database the session is actually connected to — and the
    // session composition's prompt facts, which its prompts may state.
    let primary = session.primary.clone();
    let facts = session.approval_facts(runtime);
    let decider: Arc<dyn ApprovalDecider> =
        Arc::new(crate::prompt_approval::TerminalApproval::from_session(
            policy, can_prompt, primary, facts, journal,
        ));
    let work = agent::runtime::run_prompt_with_sink(
        runtime,
        prompt,
        approval,
        can_prompt,
        overrides,
        history,
        &sink,
        cancellation.clone(),
        Some(state_db.clone()),
        Some(decider),
        None,
        Some(session),
        agent_mode,
    );
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => result.map(|output| PromptResult::Completed(Box::new(output))),
        _ = tokio::signal::ctrl_c() => {
            cancellation.cancel();
            Ok(PromptResult::Cancelled)
        }
    }
}
