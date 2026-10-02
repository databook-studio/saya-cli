//! Private tool execution policy: approval resolution and structural permits.

use crate::{AgentLimits, ApprovalDecider, LocalStateEffect, ToolDefinition};
use serde_json::Value;

/// Whether the runner must refuse to run `definition` unattended because it
/// has an external side effect. A tool that touches the world outside the
/// agent must go through approval; when it already requires approval this
/// gate is satisfied by the prompt, so the term only denies a tool that set
/// `external_side_effect` without also setting `requires_approval`. The one
/// exception is the plan-gated egress permit for a tool whose run scope
/// already approved that effect once with its plan, not per call.
pub(super) fn external_side_effect_gated(
    definition: &ToolDefinition,
    limits: &AgentLimits,
) -> bool {
    definition.effect.external_side_effect
        && !definition.effect.requires_approval
        && !limits.permit_external_effects
}

/// Whether a candidate-claim write lacks its explicit run permit.
pub(super) fn candidate_denied(definition: &ToolDefinition, limits: &AgentLimits) -> bool {
    definition.effect.local_state == LocalStateEffect::WriteCandidate
        && !limits.permit_candidate_writes
}

/// Whether a workspace-file write lacks its explicit run permit.
pub(super) fn workspace_write_denied(definition: &ToolDefinition, limits: &AgentLimits) -> bool {
    definition.effect.local_state == LocalStateEffect::WriteWorkspace
        && !limits.permit_workspace_writes
}

/// The approval state supplied to the pure execution policy.
#[derive(Clone, Copy)]
pub(super) enum ApprovalState {
    NotRequired,
    Granted,
    Denied,
}

/// The pure execution-policy outcome every tool-call path consumes.
#[derive(Clone, Copy)]
pub(super) enum ExecutionDecision {
    Allow,
    RequireApproval,
    Deny(PolicyDenial),
}

/// The structural or approval reason that denied execution.
#[derive(Clone, Copy)]
pub(super) enum PolicyDenial {
    Approval,
    ExternalSideEffect,
    CandidateWrite,
    WorkspaceWrite,
}

impl PolicyDenial {
    pub(super) const fn reason(self) -> &'static str {
        match self {
            Self::Approval => "approval was not granted",
            Self::ExternalSideEffect => "external side effect requires approval",
            Self::CandidateWrite => "candidate writes are not permitted",
            Self::WorkspaceWrite => "workspace writes are not permitted",
        }
    }
}

/// A declined approval takes wording precedence; a grant still reaches every
/// structural restriction.
pub(super) fn execution_decision(
    definition: &ToolDefinition,
    limits: &AgentLimits,
    approval: ApprovalState,
) -> ExecutionDecision {
    match approval {
        ApprovalState::NotRequired if definition.effect.requires_approval => {
            return ExecutionDecision::RequireApproval;
        }
        ApprovalState::Denied => return ExecutionDecision::Deny(PolicyDenial::Approval),
        ApprovalState::NotRequired | ApprovalState::Granted => {}
    }
    if external_side_effect_gated(definition, limits) {
        ExecutionDecision::Deny(PolicyDenial::ExternalSideEffect)
    } else if candidate_denied(definition, limits) {
        ExecutionDecision::Deny(PolicyDenial::CandidateWrite)
    } else if workspace_write_denied(definition, limits) {
        ExecutionDecision::Deny(PolicyDenial::WorkspaceWrite)
    } else {
        ExecutionDecision::Allow
    }
}

/// Resolves a required approval prompt without folding it into the execution
/// policy. Custom refusal text is observed only after an actual refusal.
pub(super) async fn resolve_approval(
    definition: &ToolDefinition,
    arguments: &Value,
    approval: &dyn ApprovalDecider,
) -> (ApprovalState, Option<String>) {
    if !definition.effect.requires_approval {
        return (ApprovalState::NotRequired, None);
    }
    if approval.approve(definition, arguments).await {
        (ApprovalState::Granted, None)
    } else {
        (
            ApprovalState::Denied,
            approval.refusal_detail(definition, arguments),
        )
    }
}
