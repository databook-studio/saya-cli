use crate::{
    AgentRequest, ApprovalDecider, CancellationToken, ChatProvider, NoopEventSink, ToolDefinition,
    ToolExecutor,
    loop_runner::{AgentLimits, AgentOutput, run_agent_with_sink},
};

pub async fn run_agent(
    provider: &dyn ChatProvider,
    tools: &dyn ToolExecutor,
    request: AgentRequest,
    definitions: Vec<ToolDefinition>,
    limits: AgentLimits,
    approval: &dyn ApprovalDecider,
) -> Result<AgentOutput, crate::AgentError> {
    run_agent_with_sink(
        provider,
        tools,
        request,
        definitions,
        limits,
        approval,
        &NoopEventSink,
        CancellationToken::new(),
    )
    .await
}
