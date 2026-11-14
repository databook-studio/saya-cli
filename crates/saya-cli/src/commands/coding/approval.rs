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
        Self::definition_for(&Value::Null)
    }

    /// The same approval, worded for the proposal it approves. An in-place job
    /// writes the operator's own folder while it runs, so its consent says so;
    /// a Git job keeps its exact published description.
    pub fn definition_for(proposal: &Value) -> ToolDefinition {
        let description = if proposal.get("mode").and_then(Value::as_str) == Some("in_place") {
            "Approve this exact job; it edits the folder in place, live, with no Git history to recover from"
        } else {
            "Approve this exact job and its bounded scope"
        };
        ToolDefinition {
            name: "coding_job".into(),
            description: description.into(),
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

#[cfg(all(test, unix))]
#[path = "approval_tests.rs"]
mod approval_tests;
