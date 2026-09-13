//! The session approval policy: the one engine every approval frontend
//! consults, so the mode match and the session-grant state live in exactly
//! one place. A frontend renders an [`ApprovalDecision::Ask`] and reports the
//! user's [`ApprovalChoice`] back; it never implements policy itself.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::ToolEffect;
use super::approval::read_only_permits;
use crate::protocol::approval::ApprovalPolicy;

/// How the engine resolves one tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Run without asking.
    Allow,
    /// Render the ask and wait for the user's [`ApprovalChoice`].
    Ask,
    /// Refuse without asking.
    Deny,
}

/// The user's answer to an ask: allow once, allow for this session, or deny.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalChoice {
    /// Approve this one call; nothing is remembered.
    AllowOnce,
    /// Approve and grant the call's suggested token for the session. Both
    /// interactive frontends produce this choice — the terminal's `[s]`
    /// answer (`prompt_approval`) and the TUI modal's `[s]` key — so the
    /// grant store holds what the user actually granted, one token per ask.
    AllowSession {
        /// The grammar token the grant records, in the `--allow` vocabulary.
        token: String,
    },
    /// Refuse.
    Deny,
}

/// The grammar tokens granted for this session: process lifetime, never
/// persisted, and a resumed session starts empty. Additive only — nothing
/// widens a grant except another explicit user choice.
#[derive(Debug, Clone, Default)]
pub struct SessionGrants {
    tokens: Arc<Mutex<BTreeSet<String>>>,
}

impl SessionGrants {
    /// Records a grant and reports whether it was new, so a first grant can
    /// be journaled exactly once.
    pub fn grant(&self, token: &str) -> bool {
        self.locked().insert(token.to_owned())
    }

    /// Whether this token is granted for the session.
    pub fn is_granted(&self, token: &str) -> bool {
        self.locked().contains(token)
    }

    /// Whether nothing has been granted yet.
    pub fn is_empty(&self) -> bool {
        self.locked().is_empty()
    }

    /// The granted tokens in sorted order — a session listing's words.
    pub fn tokens(&self) -> Vec<String> {
        self.locked().iter().cloned().collect()
    }

    /// The grant set is a plain set, so a panicked holder cannot have left it
    /// in a state recovery must fear; taking the guard anyway keeps approving
    /// (and prompting) rather than wedging the session on a poisoned lock.
    fn locked(&self) -> MutexGuard<'_, BTreeSet<String>> {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The approval engine. One instance per interactive session, shared by every
/// turn's decider: the mode is the session's `--approval-mode`, the grants
/// start empty and grow only through [`SessionPolicy::record`].
#[derive(Debug, Clone)]
pub struct SessionPolicy {
    mode: ApprovalPolicy,
    grants: SessionGrants,
}

impl SessionPolicy {
    pub fn new(mode: ApprovalPolicy) -> Self {
        Self {
            mode,
            grants: SessionGrants::default(),
        }
    }

    /// The session's grant store — what a prompt's "already allowed" facts
    /// and a session listing would read.
    pub fn grants(&self) -> &SessionGrants {
        &self.grants
    }

    /// Resolves one tool call. `grant_token` is the grammar token a session
    /// grant for this call would be recorded under, when the caller can name
    /// one; callers that do not suggest tokens pass `None` and the decision
    /// falls to the mode alone. A grant pre-answers the call the mode would
    /// have asked about, so it allows under `ask`; `read-only` and `never`
    /// never ask, so grants cannot move them — read-only still allows exactly
    /// the read-shaped tools (`read_only_permits`) and denies the rest, and
    /// `never` denies everything. `bypass` allows every call: it is the
    /// per-call consent given once, in the launch flag, so no grant is
    /// consulted and none is recorded — no ask occurs under it. The mode
    /// judges *who answers*, never *what the tool is*: every structural guard
    /// lives in the tools and the composition, untouched by any mode.
    pub fn resolve(&self, effect: &ToolEffect, grant_token: Option<&str>) -> ApprovalDecision {
        match self.mode {
            ApprovalPolicy::Bypass => ApprovalDecision::Allow,
            ApprovalPolicy::ReadOnly if read_only_permits(effect) => ApprovalDecision::Allow,
            ApprovalPolicy::ReadOnly => ApprovalDecision::Deny,
            ApprovalPolicy::Never => ApprovalDecision::Deny,
            ApprovalPolicy::Ask
                if grant_token.is_some_and(|token| self.grants.is_granted(token)) =>
            {
                ApprovalDecision::Allow
            }
            ApprovalPolicy::Ask => ApprovalDecision::Ask,
        }
    }

    /// Records the user's answer to an ask. Only "allow for this session"
    /// changes the policy, by granting its token; "allow once" and "deny"
    /// leave no state behind. Reports whether a new grant was recorded — the
    /// moment a first grant is journaled.
    pub fn record(&self, choice: ApprovalChoice) -> bool {
        match choice {
            ApprovalChoice::AllowSession { token } => self.grants.grant(&token),
            ApprovalChoice::AllowOnce | ApprovalChoice::Deny => false,
        }
    }
}
