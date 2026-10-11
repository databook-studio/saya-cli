use super::{CodingApprovalRequest, CodingError};
use async_trait::async_trait;
use saya_agent::{ApprovalDecider, LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};
use serde_json::{Value, json};

/// Exact, explicitly user-selected preauthorization for one native job. It is
/// not deserializable and is never carried into a child toolset or continuation.
pub struct CodingAuthorization {
    request: Value,
    used: std::sync::atomic::AtomicBool,
}
impl CodingAuthorization {
    pub fn scoped(proposal: &CodingApprovalRequest) -> Result<Self, CodingError> {
        Ok(Self {
            request: proposal.request().clone(),
            used: std::sync::atomic::AtomicBool::new(false),
        })
    }
    pub fn request(&self) -> Value {
        self.request.clone()
    }
    pub fn definition() -> ToolDefinition {
        ToolDefinition {
            name: "coding_job".into(),
            description: "Approve this exact native coding job and its bounded scope".into(),
            read_only: false,
            parameters: json!({"type":"object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: true,
                local_state: LocalStateEffect::WriteWorkspace,
            },
            concurrency: ToolConcurrency::Serial,
            completion: None,
        }
    }
}
#[async_trait]
impl ApprovalDecider for CodingAuthorization {
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        tool.name == "coding_job"
            && tool.effect == Self::definition().effect
            && arguments == &self.request
            && self
                .used
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_ok()
    }
}

/// A live parent's already-approved ceiling used only by its issuance owner.
/// Exact identity and proposed scopes are bound into each fresh root request.
pub(super) struct ChildIssuanceApproval {
    pub(super) expected: Value,
}
#[async_trait]
impl ApprovalDecider for ChildIssuanceApproval {
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        tool.name == "coding_root_scope"
            && tool.effect.requires_approval
            && tool.effect.local_state == LocalStateEffect::WriteWorkspace
            && arguments == &self.expected
    }
}

#[cfg(test)]
#[path = "approval_tests.rs"]
mod approval_tests;
