use crate::{agent_provider, agent_tools, config_runtime::RuntimeConfig};
use saya_agent::{AgentError, AgentLimits, AgentOutput, AgentRequest, ApprovalPolicy, run_agent};
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
}

pub(crate) async fn run_prompt(
    runtime: &RuntimeConfig,
    prompt: &str,
    approval: ApprovalPolicy,
    can_prompt: bool,
) -> Result<AgentOutput, AgentRuntimeError> {
    let provider = agent_provider::build(&runtime.resolved.ai, &runtime.secret_resolver())
        .map_err(|error| AgentRuntimeError::Provider(error.to_string()))?;
    let connector = match runtime.resolved.profile.as_ref() {
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
    let tools = agent_tools::DatabaseTools::new(connector, runtime.resolved.max_rows);
    let request = AgentRequest {
        prompt: prompt.into(),
        profile_names: runtime.resolved.profile_name.iter().cloned().collect(),
        model: runtime.resolved.ai.model.clone(),
    };
    run_agent(
        &*provider,
        &tools,
        request,
        agent_tools::DatabaseTools::definitions(),
        AgentLimits {
            max_turns: runtime.resolved.max_iterations,
            max_tool_calls: runtime.resolved.max_iterations.saturating_mul(2),
        },
        &TerminalApproval::new(approval, can_prompt),
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
    })
}

struct TerminalApproval {
    policy: ApprovalPolicy,
    can_prompt: bool,
}

impl TerminalApproval {
    fn new(policy: ApprovalPolicy, can_prompt: bool) -> Self {
        Self { policy, can_prompt }
    }
}

#[async_trait::async_trait]
impl saya_agent::ApprovalDecider for TerminalApproval {
    async fn approve(&self, _: &saya_agent::ToolDefinition) -> bool {
        match self.policy {
            ApprovalPolicy::ReadOnly => true,
            ApprovalPolicy::Never => false,
            ApprovalPolicy::Ask if !self.can_prompt => false,
            ApprovalPolicy::Ask => {
                use std::io::{self, IsTerminal, Write};
                if !io::stdin().is_terminal() {
                    return false;
                }
                eprint!("Allow bounded read-only SQL query? [y/N] ");
                let _ = io::stderr().flush();
                let mut answer = String::new();
                io::stdin().read_line(&mut answer).is_ok()
                    && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
            }
        }
    }
}
