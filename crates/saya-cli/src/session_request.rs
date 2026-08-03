use crate::{
    agent_runtime::{self, AgentRuntimeError, PromptOverrides},
    config_runtime::RuntimeConfig,
    render::RenderFormat,
    stream_render::TerminalSink,
};
use saya_agent::{AgentOutput, ApprovalPolicy, CancellationToken, ChatMessage};
use saya_store::SqliteStateStore;

pub(crate) enum PromptResult {
    Completed(AgentOutput),
    Cancelled,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    runtime: &RuntimeConfig,
    prompt: &str,
    approval: ApprovalPolicy,
    can_prompt: bool,
    overrides: PromptOverrides,
    history: Vec<ChatMessage>,
    format: RenderFormat,
    state_db: &SqliteStateStore,
) -> Result<PromptResult, AgentRuntimeError> {
    let cancellation = CancellationToken::new();
    let sink = TerminalSink::new(format);
    let work = agent_runtime::run_prompt_with_sink(
        runtime,
        prompt,
        approval,
        can_prompt,
        overrides,
        history,
        &sink,
        cancellation.clone(),
        Some(state_db.clone()),
    );
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => result.map(PromptResult::Completed),
        _ = tokio::signal::ctrl_c() => {
            cancellation.cancel();
            Ok(PromptResult::Cancelled)
        }
    }
}
