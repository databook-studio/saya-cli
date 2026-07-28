use crate::{
    agent_provider, agent_tools, config_runtime::RuntimeConfig, prompt_approval::TerminalApproval,
};
use saya_agent::{
    AgentError, AgentEventSink, AgentLimits, AgentOutput, AgentRequest, ApprovalPolicy,
    CancellationToken, ChatMessage, run_agent_with_sink,
};
use saya_config::{AiProvider, ResolvedAi};
use saya_connectors::{ConnectorOptions, build_connector};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum AgentRuntimeError {
    #[error("{0}")]
    Provider(String),
    #[error("{0}")]
    Database(String),
    #[error("{0}")]
    Agent(String),
    #[error("{0}")]
    Configuration(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PromptOverrides {
    pub(crate) provider: Option<AiProvider>,
    pub(crate) model: Option<String>,
    pub(crate) allow_data_sharing: Option<bool>,
    pub(crate) profile: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_prompt_with_sink(
    runtime: &RuntimeConfig,
    prompt: &str,
    approval: ApprovalPolicy,
    can_prompt: bool,
    overrides: PromptOverrides,
    history: Vec<ChatMessage>,
    sink: &dyn AgentEventSink,
    cancellation: CancellationToken,
) -> Result<AgentOutput, AgentRuntimeError> {
    let ai = effective_ai(&runtime.resolved.ai, &overrides);
    let provider = agent_provider::build(&ai, &runtime.secret_resolver())
        .map_err(|error| AgentRuntimeError::Provider(error.to_string()))?;
    let (profile_name, profile) = selected_profile(runtime, overrides.profile.as_ref())?;
    let allow_query_data = query_data_allowed(ai.provider, ai.allow_data_sharing);
    let connector = match profile.as_ref() {
        Some(profile) => Some(
            build_connector(
                profile,
                &runtime.secret_resolver(),
                ConnectorOptions {
                    query_timeout_seconds: runtime.resolved.query_timeout_seconds,
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| AgentRuntimeError::Database(error.to_string()))?,
        ),
        None => None,
    };
    if let Some(connector) = connector.as_ref() {
        connector
            .connect()
            .await
            .map_err(|error| AgentRuntimeError::Database(error.to_string()))?;
    }
    let tools =
        agent_tools::DatabaseTools::new(connector, runtime.resolved.max_rows, allow_query_data);
    let request = AgentRequest {
        prompt: prompt.into(),
        profile_names: profile_name.into_iter().collect(),
        model: ai.model,
        history,
    };
    run_agent_with_sink(
        &*provider,
        &tools,
        request,
        agent_tools::DatabaseTools::definitions(allow_query_data),
        AgentLimits {
            max_turns: runtime.resolved.max_iterations,
            max_tool_calls: runtime.resolved.max_iterations.saturating_mul(2),
        },
        &TerminalApproval::new(approval, can_prompt),
        sink,
        cancellation,
    )
    .await
    .map_err(|error| match error {
        AgentError::Provider(error) => AgentRuntimeError::Provider(error.to_string()),
        AgentError::Limit(error) => {
            AgentRuntimeError::Agent(format!("agent limit reached: {error}"))
        }
        AgentError::InvalidToolCall => {
            AgentRuntimeError::Agent("provider returned an unsupported tool call".into())
        }
        AgentError::InvalidHistory => {
            AgentRuntimeError::Agent("conversation history is invalid".into())
        }
        AgentError::ContextLimit => {
            AgentRuntimeError::Agent("conversation context exceeds the safe limit".into())
        }
        AgentError::Cancelled => AgentRuntimeError::Agent("request cancelled".into()),
    })
}

pub(crate) fn query_data_allowed(provider: AiProvider, allow_data_sharing: bool) -> bool {
    match provider {
        AiProvider::Openai | AiProvider::OpenaiCompatible => allow_data_sharing,
        AiProvider::Ollama => true,
        _ => false,
    }
}

pub(crate) fn effective_ai(base: &ResolvedAi, overrides: &PromptOverrides) -> ResolvedAi {
    let mut ai = base.clone();
    if let Some(provider) = overrides.provider {
        if ai.provider != provider {
            ai.base_url = None;
        }
        ai.provider = provider;
    }
    if let Some(model) = overrides.model.as_ref() {
        ai.model = model.clone();
    }
    if let Some(value) = overrides.allow_data_sharing {
        ai.allow_data_sharing = value;
    }
    ai
}

fn selected_profile(
    runtime: &RuntimeConfig,
    override_name: Option<&String>,
) -> Result<(Option<String>, Option<saya_types::DatabaseProfile>), AgentRuntimeError> {
    match override_name {
        Some(name) => runtime
            .named_profile(name)
            .map(|profile| (Some(name.clone()), Some(profile.clone())))
            .map_err(|error| AgentRuntimeError::Configuration(error.to_string())),
        None => Ok((
            runtime.resolved.profile_name.clone(),
            runtime.resolved.profile.clone(),
        )),
    }
}
