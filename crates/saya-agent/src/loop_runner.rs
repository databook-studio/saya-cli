use crate::{
    AgentEvent, AgentRequest, ApprovalDecider, ChatMessage, ChatProvider, ChatRequest,
    ProviderError, ToolDefinition, ToolExecutor,
};
use serde_json::Value;
use thiserror::Error;

const SYSTEM_PROMPT: &str = "You are SAYA, a database assistant. Use only the supplied read-only tools. Never claim to have written data or used unsupported tools.";

#[derive(Debug, Clone, Copy)]
pub struct AgentLimits {
    pub max_turns: usize,
    pub max_tool_calls: usize,
}

impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            max_turns: 12,
            max_tool_calls: 24,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOutput {
    pub answer: String,
    pub events: Vec<AgentEvent>,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("{0}")]
    Provider(#[from] ProviderError),
    #[error("agent limit reached: {0}")]
    Limit(&'static str),
    #[error("provider returned an unsupported tool call")]
    InvalidToolCall,
}

pub async fn run_agent(
    provider: &dyn ChatProvider,
    tools: &dyn ToolExecutor,
    request: AgentRequest,
    definitions: Vec<ToolDefinition>,
    limits: AgentLimits,
    approval: &dyn ApprovalDecider,
) -> Result<AgentOutput, AgentError> {
    let mut messages = vec![ChatMessage::text("system", SYSTEM_PROMPT)];
    messages.push(ChatMessage::text("user", request.prompt));
    let model = request.model;
    let mut events = Vec::new();
    let mut tool_calls = 0;
    for _ in 0..limits.max_turns {
        let response = provider
            .complete(ChatRequest {
                model: model.clone(),
                messages: messages.clone(),
                tools: definitions.clone(),
            })
            .await?;
        let assistant = response.message;
        messages.push(assistant.clone());
        if assistant.tool_calls.is_empty() {
            events.push(AgentEvent::AssistantText {
                text: assistant.content.clone(),
            });
            events.push(AgentEvent::Complete);
            return Ok(AgentOutput {
                answer: assistant.content,
                events,
            });
        }
        for call in assistant.tool_calls {
            if call.name.is_empty()
                || !call.arguments.is_object()
                || !definitions.iter().any(|tool| tool.name == call.name)
            {
                return Err(AgentError::InvalidToolCall);
            }
            tool_calls += 1;
            if tool_calls > limits.max_tool_calls {
                return Err(AgentError::Limit("tool calls"));
            }
            events.push(AgentEvent::ToolRequested {
                name: call.name.clone(),
            });
            let definition = definitions
                .iter()
                .find(|tool| tool.name == call.name)
                .unwrap();
            let approved = !definition.requires_approval || approval.approve(definition).await;
            let (result, summary) = if approved {
                match tools.execute(&call.name, call.arguments).await {
                    Ok(result) => (result, "read-only database tool completed"),
                    Err(_) => (
                        serde_json::json!({"error":"read-only database tool failed"}),
                        "read-only database tool failed",
                    ),
                }
            } else {
                events.push(AgentEvent::ToolDenied {
                    name: call.name.clone(),
                    reason: "approval was not granted".into(),
                });
                (
                    serde_json::json!({"error":"tool call denied by approval policy"}),
                    "read-only database tool denied",
                )
            };
            let content = bounded_json(&result);
            messages.push(ChatMessage {
                role: "tool".into(),
                content,
                tool_calls: Vec::new(),
                tool_call_id: Some(call.id),
            });
            if approved {
                events.push(AgentEvent::ToolCompleted {
                    name: call.name,
                    summary: summary.into(),
                });
            }
        }
    }
    Err(AgentError::Limit("turns"))
}

fn bounded_json(value: &Value) -> String {
    let text = serde_json::to_string(value)
        .unwrap_or_else(|_| "{\"error\":\"tool result unavailable\"}".into());
    if text.len() <= 65_536 {
        text
    } else {
        "{\"error\":\"tool result exceeded model context limit\"}".into()
    }
}
