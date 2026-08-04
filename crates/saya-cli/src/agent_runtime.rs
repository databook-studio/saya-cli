use crate::{
    agent_provider, agent_tools, config_runtime::RuntimeConfig, prompt_approval::TerminalApproval,
};
use saya_agent::{
    AgentError, AgentEventSink, AgentLimits, AgentOutput, AgentRequest, ApprovalPolicy,
    CancellationToken, ChatMessage, run_agent_with_sink,
};
use saya_config::{AiProvider, ResolvedAi};
use saya_store::SqliteStateStore;
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
    pub(crate) included_profiles: Vec<String>,
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
    state_db: Option<SqliteStateStore>,
) -> Result<AgentOutput, AgentRuntimeError> {
    let ai = effective_ai(&runtime.resolved.ai, &overrides);
    let provider = agent_provider::build(&ai, &runtime.secret_resolver())
        .map_err(|error| AgentRuntimeError::Provider(error.to_string()))?;
    let (profile_name, profile) =
        crate::agent_profile::selected(runtime, overrides.profile.as_ref())?;
    let allow_query_data = query_data_allowed(ai.provider, ai.allow_data_sharing);

    let mut secondaries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in &overrides.included_profiles {
        if name.is_empty() {
            continue;
        }
        if profile_name.as_deref() == Some(name.as_str()) {
            continue;
        }
        if !seen.insert(name) {
            continue;
        }
        if let Ok(sec_profile) = runtime.named_profile(name) {
            secondaries.push((name.clone(), sec_profile.clone()));
        }
    }

    let registry = match profile.as_ref() {
        Some(primary_profile) => {
            let primary_name = profile_name.as_deref().unwrap_or("");
            crate::connection_build::build_registry(
                &runtime.secret_resolver(),
                &runtime.cache_scope,
                runtime.resolved.query_timeout_seconds,
                can_prompt,
                primary_name,
                primary_profile,
                &secondaries,
            )
            .await?
        }
        None => crate::connection_registry::ConnectionRegistry::new(""),
    };

    let system_prompt = registry.describe_context();
    let profile_names: Vec<String> = registry.names().into_iter().map(str::to_string).collect();
    let tools = agent_tools::DatabaseTools::with_registry(
        registry,
        runtime.resolved.max_rows,
        allow_query_data,
        state_db,
    );
    let request = AgentRequest {
        prompt: prompt.into(),
        profile_names,
        model: ai.model,
        system_prompt,
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
        AiProvider::Openai
        | AiProvider::OpenaiCompatible
        | AiProvider::Anthropic
        | AiProvider::Gemini => allow_data_sharing,
        AiProvider::Ollama => true,
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
