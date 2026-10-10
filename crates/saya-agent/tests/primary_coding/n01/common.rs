use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, AgentRunControl, AnsweringLimits, ApprovalDecider, BudgetDimension,
    CancellationReason, CancellationToken, LocalStateEffect, NoopEventSink, ToolConcurrency,
    ToolDefinition, ToolEffect,
};
use std::sync::atomic::{AtomicUsize, Ordering};

pub static NOOP: NoopEventSink = NoopEventSink;

pub struct ApproveAll {
    pub calls: AtomicUsize,
}

impl Default for ApproveAll {
    fn default() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ApprovalDecider for ApproveAll {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        true
    }
}

pub struct CancelBudgetApproval(pub CancellationToken);

#[async_trait]
impl ApprovalDecider for CancelBudgetApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        self.0
            .cancel_with_reason(CancellationReason::Budget(BudgetDimension::LogicalRequests));
        true
    }
}

pub fn request() -> AgentRequest {
    AgentRequest {
        prompt: "run the requested task".to_owned(),
        profile_names: Vec::new(),
        model: "test-model".to_owned(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

pub fn limits() -> (AgentLimits, AnsweringLimits) {
    (
        AgentLimits {
            max_turns: Some(4),
            max_tool_calls: Some(8),
            max_continuations: Some(0),
            permit_candidate_writes: true,
            permit_workspace_writes: true,
            permit_external_effects: true,
            context_byte_budget: 64 * 1024,
        },
        AnsweringLimits {
            max_requests: Some(4),
            max_tool_calls: Some(8),
            max_reported_tokens: Some(10_000),
        },
    )
}

pub fn control<'a>(
    approval: &'a dyn ApprovalDecider,
    cancellation: CancellationToken,
) -> AgentRunControl<'a> {
    let (agent, answering) = limits();
    AgentRunControl::new(agent, answering, approval, &NOOP, cancellation)
}

pub fn effect(read: bool, external: bool, approval: bool) -> ToolEffect {
    ToolEffect {
        database_data: read,
        external_side_effect: external,
        requires_approval: approval,
        local_state: if read {
            LocalStateEffect::Read
        } else {
            LocalStateEffect::None
        },
    }
}

pub fn tool(name: &str, effect: ToolEffect, concurrency: ToolConcurrency) -> ToolDefinition {
    ToolDefinition {
        name: name.to_owned(),
        description: "bounded test action".to_owned(),
        read_only: !effect.external_side_effect,
        parameters: serde_json::json!({"type": "object"}),
        effect,
        concurrency,
        completion: None,
    }
}

pub fn tool_call(name: &str, id: &str) -> saya_agent::ToolCall {
    saya_agent::ToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        arguments: serde_json::json!({"value": "fixed"}),
    }
}
