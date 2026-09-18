//! Tool-call approval: the decider trait a caller implements to gate a tool
//! call, and the read-only-policy implementation.

use std::{fmt, str::FromStr};

use async_trait::async_trait;

use super::session_policy::{ApprovalDecision, SessionPolicy};
use super::{LocalStateEffect, ToolDefinition, ToolEffect};
use crate::protocol::approval::ApprovalPolicy;

/// The agent's task posture: `build` does the work, `plan` investigates and
/// answers with a plan — write-shaped tools refuse before the approval match
/// runs. Orthogonal to [`ApprovalPolicy`], which is the consent posture: the
/// mode narrows, never widens, so `plan` denies writes under every policy
/// including `bypass`, and a grant made in `build` is inert under `plan` and
/// live again on return.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentMode {
    /// Do the work: resolve exactly as the approval policy says.
    #[default]
    Build,
    /// Investigate and answer with a plan: refuse write-shaped calls before
    /// the approval match runs, whatever the policy would have said.
    Plan,
}

impl AgentMode {
    /// The mode's own spelling, rendered wherever the mode is named.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
        }
    }
}

impl FromStr for AgentMode {
    type Err = AgentModeParseError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "build" => Ok(Self::Build),
            "plan" => Ok(Self::Plan),
            _ => Err(AgentModeParseError(value.into())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentModeParseError(String);

impl fmt::Display for AgentModeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid agent mode: {}", self.0)
    }
}
impl std::error::Error for AgentModeParseError {}

#[async_trait]
pub trait ApprovalDecider: Send + Sync {
    /// Decides whether a tool call may run. `arguments` is the raw call payload
    /// so implementations can show the user what they are approving.
    async fn approve(&self, tool: &ToolDefinition, arguments: &serde_json::Value) -> bool;

    /// The typed refusal the model should relay when [`approve`] denied this
    /// exact call — `None` when the decider carries no per-call wording and
    /// the loop's generic denial stands. Called only after a `false` verdict,
    /// so a decider observes each refused call twice (once to decide, once to
    /// word); deciders must keep this side-effect free — no prompt, no grant,
    /// no journal — because the loop calls it on every denial path.
    fn refusal_detail(
        &self,
        _tool: &ToolDefinition,
        _arguments: &serde_json::Value,
    ) -> Option<String> {
        None
    }
}

/// Whether read-only approval may auto-approve a tool with this effect: only
/// read-shaped tools — no external side effect, no local-state write. Needing
/// approval is not side-effecting, so the SQL tools (`requires_approval: true,
/// external_side_effect: false`) stay auto-approved under read-only.
pub fn read_only_permits(effect: &ToolEffect) -> bool {
    !effect.external_side_effect
        && matches!(
            effect.local_state,
            LocalStateEffect::None | LocalStateEffect::Read
        )
}

/// The read-only approval policy: auto-approves read-shaped tools, denies
/// everything else. Decides through the same engine every approval frontend
/// consults — its read-only arm is the policy engine's read-only mode with
/// nothing granted.
pub struct AllowReadOnlyApproval;

#[async_trait]
impl ApprovalDecider for AllowReadOnlyApproval {
    async fn approve(&self, tool: &ToolDefinition, _: &serde_json::Value) -> bool {
        SessionPolicy::new(ApprovalPolicy::ReadOnly).resolve(&tool.effect, None)
            == ApprovalDecision::Allow
    }
}
