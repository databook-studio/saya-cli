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
    /// Refuse without asking. `reason` names why when the refusal carries
    /// information beyond the mode itself — a headless ask no reader can
    /// answer names the run's real approval. `None` is the mode's own
    /// refusal, whose words the mode's documentation already states.
    Deny {
        /// Why the call was refused, when there is more to say than the mode.
        reason: Option<&'static str>,
    },
}

/// Why a headless `Ask` denies: a run's approval surface is its `--allow`
/// scopes — typed, stated before anything runs — and a headless surface has
/// no reader an ask could reach. This is the engine's own wording for the
/// denial, not a frontend's.
pub const HEADLESS_ASK_REASON: &str =
    "cannot prompt: a headless run's approval is its `--allow` scopes";

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
/// start empty and grow only through [`SessionPolicy::record`]. The frozen
/// shape ([`SessionPolicy::frozen`]) is the headless run's: seeded from
/// `--allow`, unable to ask, unable to accumulate.
#[derive(Debug, Clone)]
pub struct SessionPolicy {
    mode: ApprovalPolicy,
    grants: SessionGrants,
    /// Whether the policy accepts answers. `false` — the interactive shape —
    /// asks and records; `true` — the headless run's — resolves what its
    /// seeds do not cover to a structured deny and refuses every grant.
    frozen: bool,
}

impl SessionPolicy {
    pub fn new(mode: ApprovalPolicy) -> Self {
        Self {
            mode,
            grants: SessionGrants::default(),
            frozen: false,
        }
    }

    /// The headless run's policy, frozen: seeded from the run's `--allow`
    /// tokens — the stated scopes are the approval — and unable to ask or
    /// accumulate. An `Ask` the seeds do not cover resolves to a structured
    /// deny naming [`HEADLESS_ASK_REASON`]; `record` refuses, so a headless
    /// session grant is impossible, not merely unused.
    pub fn frozen(mode: ApprovalPolicy, seeds: &[String]) -> Self {
        let policy = Self {
            mode,
            grants: SessionGrants::default(),
            frozen: true,
        };
        for token in seeds {
            policy.grants.grant(token);
        }
        policy
    }

    /// The policy's mode — what a session listing reads to state the mode
    /// the store sits under.
    pub fn mode(&self) -> ApprovalPolicy {
        self.mode
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
    /// consulted and none is recorded — no ask occurs under it. On a frozen
    /// policy — the headless run's — an `Ask` the seeds do not cover resolves
    /// to [`ApprovalDecision::Deny`] naming [`HEADLESS_ASK_REASON`], never to
    /// an ask: a headless surface has no reader, and its approval is its
    /// seeds. The mode judges *who answers*, never *what the tool is*: every
    /// structural guard lives in the tools and the composition, untouched by
    /// any mode.
    pub fn resolve(&self, effect: &ToolEffect, grant_token: Option<&str>) -> ApprovalDecision {
        match self.mode {
            ApprovalPolicy::Bypass => ApprovalDecision::Allow,
            ApprovalPolicy::ReadOnly if read_only_permits(effect) => ApprovalDecision::Allow,
            ApprovalPolicy::ReadOnly => ApprovalDecision::Deny { reason: None },
            ApprovalPolicy::Never => ApprovalDecision::Deny { reason: None },
            ApprovalPolicy::Ask
                if grant_token.is_some_and(|token| self.grants.is_granted(token)) =>
            {
                ApprovalDecision::Allow
            }
            ApprovalPolicy::Ask if self.frozen => ApprovalDecision::Deny {
                reason: Some(HEADLESS_ASK_REASON),
            },
            ApprovalPolicy::Ask => ApprovalDecision::Ask,
        }
    }

    /// Records the user's answer to an ask. Only "allow for this session"
    /// changes the policy, by granting its token; "allow once" and "deny"
    /// leave no state behind. Reports whether a new grant was recorded — the
    /// moment a first grant is journaled. A frozen policy — the headless
    /// run's — refuses every answer: no ask it produces could be answered,
    /// so nothing can move its store, and a headless session grant is
    /// impossible by construction.
    pub fn record(&self, choice: ApprovalChoice) -> bool {
        if self.frozen {
            return false;
        }
        match choice {
            ApprovalChoice::AllowSession { token } => self.grants.grant(&token),
            ApprovalChoice::AllowOnce | ApprovalChoice::Deny => false,
        }
    }
}
