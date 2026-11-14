//! Tool-call approval: the decider trait a caller implements to gate a tool
//! call, and the read-only-policy implementation.

use std::{fmt, str::FromStr};

use async_trait::async_trait;

use super::session_policy::{ApprovalDecision, SessionPolicy};
use super::{LocalStateEffect, ToolDefinition, ToolEffect};
use crate::protocol::approval::ApprovalPolicy;

/// A stable class supplied only for an exact immutable child-authority denial.
/// It is context, never an approval or a new grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImmutableAuthorityRefusal {
    OutsideEffectiveWritePaths,
}

impl ImmutableAuthorityRefusal {
    pub const fn class(self) -> &'static str {
        match self {
            Self::OutsideEffectiveWritePaths => "outside_effective_write_paths",
        }
    }
}

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

    /// The byte this posture occupies in a [`SessionPolicy`]'s mode cell —
    /// the same one-byte encoding [`ApprovalPolicy::code`] uses, written and
    /// read only by that type's accessors.
    ///
    /// [`SessionPolicy`]: super::session_policy::SessionPolicy
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Build => 0,
            Self::Plan => 1,
        }
    }

    /// The posture a cell byte holds. Only [`code`](Self::code) writes these
    /// bytes, so the fallback is unreachable; it reads as `build`, the
    /// posture a cell that somehow held something else would have had.
    pub(crate) const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Plan,
            _ => Self::Build,
        }
    }
}

/// The grammar token a workspace file write is granted under: the token
/// `grant_token` suggests for `write` and `edit`, and the one
/// [`ApprovalPolicy::AcceptEdits`] pre-answers. One string, one meaning — the
/// `--allow` vocabulary's own spelling, shared by the suggestion, the
/// advertisement and the engine rather than repeated in each.
pub const WORKSPACE_WRITE_TOKEN: &str = "workspace-write";

/// The session's permission posture as one switch: what `/mode` and the
/// keyboard cycle set and what a status bar shows. It is a *projection* of the
/// engine's two axes, never a second engine — every mode decomposes into an
/// [`ApprovalPolicy`] and an [`AgentMode`], and every call still resolves
/// through [`SessionPolicy::resolve`], so no mode is a path around the
/// approval match.
///
/// [`SessionPolicy::resolve`]: super::session_policy::SessionPolicy::resolve
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PermissionMode {
    /// Ask per call. The launch default, and the posture a session degrades to
    /// when nothing else is stated.
    #[default]
    Ask,
    /// Ask per call, except the workspace file family — the calls carrying
    /// [`WORKSPACE_WRITE_TOKEN`] — which run unasked.
    AcceptEdits,
    /// Investigate and answer with a plan: the write surface refuses before
    /// the approval match, and the consent posture is left as it was found.
    Plan,
    /// Every call runs without asking.
    Bypass,
}

impl PermissionMode {
    /// The pair this mode sets. A `None` consent posture means the mode leaves
    /// that axis alone: `plan` is a task posture, orthogonal to who answers,
    /// so switching into it keeps the consent the session already had.
    pub const fn decompose(self) -> (Option<ApprovalPolicy>, AgentMode) {
        match self {
            Self::Ask => (Some(ApprovalPolicy::Ask), AgentMode::Build),
            Self::AcceptEdits => (Some(ApprovalPolicy::AcceptEdits), AgentMode::Build),
            Self::Plan => (None, AgentMode::Plan),
            Self::Bypass => (Some(ApprovalPolicy::Bypass), AgentMode::Build),
        }
    }

    /// The mode a `(policy, agent_mode)` pair projects to, or `None` where the
    /// pair is not one of the four. The task axis is read first — a Plan
    /// session is `plan` whatever its consent posture. `read-only` and `never`
    /// are consent postures with no mode of their own, so they project to
    /// nothing and a bar keeps showing the pair rather than inventing a mode.
    pub const fn of(policy: ApprovalPolicy, agent_mode: AgentMode) -> Option<Self> {
        match agent_mode {
            AgentMode::Plan => Some(Self::Plan),
            AgentMode::Build => match policy {
                ApprovalPolicy::Ask => Some(Self::Ask),
                ApprovalPolicy::AcceptEdits => Some(Self::AcceptEdits),
                ApprovalPolicy::Bypass => Some(Self::Bypass),
                ApprovalPolicy::ReadOnly | ApprovalPolicy::Never => None,
            },
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

    /// Whether [`approve`] would put this call to the operator. `Some(true)`
    /// means an ask; `Some(false)` means the decision is automatic; `None`
    /// means this decider cannot say. A permission hook runs only where this
    /// answers `true`, so it never stands in front of a decision the operator
    /// was not going to be asked about. The default is `None`.
    fn would_ask(&self, _tool: &ToolDefinition, _arguments: &serde_json::Value) -> Option<bool> {
        None
    }

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

    /// The default deliberately excludes ordinary approval choices and all
    /// deciders that do not own immutable child authority.
    fn immutable_authority_refusal(
        &self,
        _tool: &ToolDefinition,
        _arguments: &serde_json::Value,
    ) -> Option<ImmutableAuthorityRefusal> {
        None
    }
}

/// Whether read-only approval may auto-approve a tool with this effect: only
/// read-shaped tools — no external side effect, no local-state write beyond
/// session metadata. Session metadata is not the state the read-only posture
/// protects: a read-only or Plan session that could not record what it is
/// doing would be absurd, and [`LocalStateEffect::WriteSession`] cannot reach
/// anything the posture guards — workspace files, the database, contracts, or
/// the cache. Needing approval is not side-effecting, so the SQL tools
/// (`requires_approval: true, external_side_effect: false`) stay auto-approved
/// under read-only. The `external_side_effect` half is unchanged: a
/// session-writing tool that *also* has an external effect is still refused.
pub fn read_only_permits(effect: &ToolEffect) -> bool {
    !effect.external_side_effect
        && matches!(
            effect.local_state,
            LocalStateEffect::None | LocalStateEffect::Read | LocalStateEffect::WriteSession
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

#[cfg(test)]
#[path = "permission_mode_tests.rs"]
mod permission_mode_tests;
