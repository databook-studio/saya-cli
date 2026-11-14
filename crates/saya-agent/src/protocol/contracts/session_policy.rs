//! The session approval policy: the one engine every approval frontend
//! consults, so the mode match and the session-grant state live in exactly
//! one place. A frontend renders an [`ApprovalDecision::Ask`] and reports the
//! user's [`ApprovalChoice`] back; it never implements policy itself.
//!
//! The engine holds two axes. The *consent posture* ([`ApprovalPolicy`]) is
//! who answers — `ask` and `accept-edits` put calls to a frontend, `read-only`
//! and `never` decide without one, `bypass` allows outright. The *task
//! posture* ([`AgentMode`]) is what the task may touch: `plan` refuses
//! write-shaped calls before the approval match runs. The task posture
//! narrows, never widens, so `plan` denies writes under every policy including
//! `bypass`, and a grant made in `build` is inert under `plan` and live again
//! on return.
//!
//! Both axes are shared cells: a clone of the policy — every turn's decider —
//! reads the mode a mid-session switch writes at its next `resolve`, and a
//! `frozen` policy (the headless run's) refuses every switch, because its
//! approval is stated before the run starts.
//!
//! The task posture narrows, with one door: a session that composes a plan
//! gate ([`PlanGateCell`]) opens `plan`'s write surface by approving a
//! submitted plan, and that one cell is what both this engine and the
//! surface's advertisement read — so an advertised write is a write the
//! engine admits, and a hidden one is refused. No gate is the closed shape.
//!
//! Every structural guard — the SQL safety layer, the sandbox, the allowlists
//! — lives in the tools and the composition, untouched by any mode: a mode
//! judges *what the task may touch* and *who answers*, never *what a tool is*.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use super::ToolEffect;
use super::approval::{AgentMode, WORKSPACE_WRITE_TOKEN, read_only_permits};
use super::session_grants::SessionGrants;
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
    /// refusal, whose words the mode's documentation states.
    Deny {
        /// Why the call was refused, when there is more to say.
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

/// The plan posture's one cell: whether an operator has approved a submitted
/// plan for this session.
///
/// Shared by construction — the surface that advertises the write family and
/// the engine that admits the calls it lists hold the same cell — so the
/// advertisement and the enforcement cannot drift into the
/// advertised-but-refused shape. The gate opens once and is closed again when
/// the session leaves `plan`; a policy composed with no cell is the closed
/// shape.
#[derive(Debug, Clone, Default)]
pub struct PlanGateCell(Arc<AtomicBool>);

impl PlanGateCell {
    /// Whether the operator has opened the plan posture's write surface.
    pub fn is_open(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Opens it: the operator approved a submitted plan.
    pub fn open(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Closes it again: the session left `plan`, so the approval that opened
    /// it — which belonged to the plan the operator read — no longer stands.
    pub fn close(&self) {
        self.0.store(false, Ordering::Release);
    }
}

/// The approval engine. One instance per interactive session, shared by every
/// turn's decider: the mode is the session's `--approval-mode`, the grants
/// start empty and grow only through [`SessionPolicy::record`]. The frozen
/// shape ([`SessionPolicy::frozen`]) is the headless run's.
#[derive(Debug, Clone)]
pub struct SessionPolicy {
    /// The consent posture, as a shared cell: a clone reads what a switch
    /// writes. `Relaxed` is enough — the cell publishes nothing beside itself,
    /// and a stale read only resolves one call as it did a moment earlier.
    mode: Arc<AtomicU8>,
    /// The task posture, a shared cell for the same reason.
    agent_mode: Arc<AtomicU8>,
    /// The plan posture's gate, when the composing surface has one. `None` —
    /// the headless run's policy, and every policy composed without a gate —
    /// leaves `plan` denying write-shaped calls exactly as it did before the
    /// gate existed.
    plan_gate: Option<PlanGateCell>,
    grants: SessionGrants,
    /// Whether the policy accepts answers. `false` — the interactive shape —
    /// asks and records; `true` — the headless run's — denies what its seeds
    /// do not cover, and refuses every grant and every switch.
    frozen: bool,
}

impl SessionPolicy {
    pub fn new(mode: ApprovalPolicy) -> Self {
        Self {
            mode: Arc::new(AtomicU8::new(mode.code())),
            agent_mode: Arc::new(AtomicU8::new(AgentMode::Build.code())),
            plan_gate: None,
            grants: SessionGrants::default(),
            frozen: false,
        }
    }

    /// Binds the session's plan gate, at construction: the cell an approved
    /// `submit_plan` opens. The composing surface holds the same cell for its
    /// advertisement, so the two read one answer. Construction, not a switch —
    /// a gate is composed once per session and survives every mode change.
    pub fn with_plan_gate(self, gate: Option<PlanGateCell>) -> Self {
        Self {
            plan_gate: gate,
            ..self
        }
    }

    /// Whether the plan posture's write surface is open. No gate is the closed
    /// shape: `plan` denies a write-shaped call exactly as it always did.
    pub fn plan_gate_open(&self) -> bool {
        self.plan_gate.as_ref().is_some_and(PlanGateCell::is_open)
    }

    /// Sets the agent's task posture at construction, leaving the approval
    /// policy and the grants untouched — a grant made in `Build` rides along
    /// inert under `Plan` and answers again on return. Construction, not a
    /// switch: a frozen policy is composed with its posture this way (the
    /// headless `plan` run), which is why it does not consult the freeze.
    /// [`set_agent_mode`](Self::set_agent_mode) is the mid-session one.
    pub fn with_agent_mode(self, agent_mode: AgentMode) -> Self {
        self.agent_mode.store(agent_mode.code(), Ordering::Relaxed);
        self
    }

    /// The headless run's policy, frozen: seeded from the run's `--allow`
    /// tokens — the stated scopes are the approval — and unable to ask,
    /// accumulate, or switch. `record` refuses, so a headless session grant
    /// is impossible, not merely unused.
    pub fn frozen(mode: ApprovalPolicy, seeds: &[String]) -> Self {
        let policy = Self {
            frozen: true,
            ..Self::new(mode)
        };
        for token in seeds {
            policy.grants.grant(token);
        }
        policy
    }

    /// Swaps the consent posture mid-session — what `/mode` and the keyboard
    /// cycle set. Refused on a frozen policy: a headless run's approval is
    /// stated before it starts and nothing may move it. Reports whether the
    /// mode was set, so a caller can say a refusal rather than assume it.
    /// Every clone of this policy reads the new mode at its next `resolve`.
    pub fn set_mode(&self, mode: ApprovalPolicy) -> bool {
        if self.frozen {
            return false;
        }
        self.mode.store(mode.code(), Ordering::Relaxed);
        true
    }

    /// Swaps the task posture mid-session, under the same refusal as
    /// [`set_mode`](Self::set_mode).
    pub fn set_agent_mode(&self, agent_mode: AgentMode) -> bool {
        if self.frozen {
            return false;
        }
        self.agent_mode.store(agent_mode.code(), Ordering::Relaxed);
        true
    }

    /// The consent posture — what a session listing reads to state the mode.
    pub fn mode(&self) -> ApprovalPolicy {
        ApprovalPolicy::from_code(self.mode.load(Ordering::Relaxed))
    }

    /// The task posture — what a session listing reads to state build or plan.
    pub fn agent_mode(&self) -> AgentMode {
        AgentMode::from_code(self.agent_mode.load(Ordering::Relaxed))
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
    /// have asked about, so it allows under `ask` — and counts the call
    /// under its token, the session-history figure a prompt reads.
    /// `read-only` and `never` never ask, so grants cannot move them —
    /// read-only still allows exactly the read-shaped tools
    /// (`read_only_permits`) and denies the rest, and `never` denies
    /// everything. `bypass` allows every call: it is the per-call consent
    /// given once, in the launch flag, so no grant is consulted and none is
    /// recorded — no ask occurs under it. `accept-edits` is `ask` plus one
    /// pre-answer of its own, the workspace file family's token
    /// ([`WORKSPACE_WRITE_TOKEN`]), which the mode states rather than the
    /// user; it records nothing, so the store holds only what the user gave.
    ///
    /// On a frozen policy — the headless run's — an `Ask` the seeds do not
    /// cover resolves to [`ApprovalDecision::Deny`] naming
    /// [`HEADLESS_ASK_REASON`], never to an ask: a headless surface has no
    /// reader, and its approval is its seeds. The freeze also refuses the
    /// `accept-edits` pre-answer, which is a reader's consent and so cannot
    /// apply where there is no reader — a run spelling that posture falls
    /// back to `ask` and denies what nothing seeded. The task posture
    /// narrows here too: under [`AgentMode::Plan`] a call whose effect fails
    /// [`read_only_permits`] denies before the approval match runs — under
    /// every policy including `bypass` — so it consults no grant and records
    /// no call. The one door is the plan gate: a session whose operator
    /// approved a submitted plan ([`PlanGateCell::open`]) resolves those calls
    /// as the consent posture alone would, which is exactly the surface the
    /// advertisement then lists.
    pub fn resolve(&self, effect: &ToolEffect, grant_token: Option<&str>) -> ApprovalDecision {
        if self.agent_mode() == AgentMode::Plan
            && !read_only_permits(effect)
            && !self.plan_gate_open()
        {
            return ApprovalDecision::Deny { reason: None };
        }
        match self.mode() {
            ApprovalPolicy::Bypass => ApprovalDecision::Allow,
            ApprovalPolicy::ReadOnly if read_only_permits(effect) => ApprovalDecision::Allow,
            ApprovalPolicy::ReadOnly => ApprovalDecision::Deny { reason: None },
            ApprovalPolicy::Never => ApprovalDecision::Deny { reason: None },
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => self.asking_arm(grant_token),
        }
    }

    /// The arm both asking postures share: a held grant pre-answers the call,
    /// a frozen policy denies with the run's own wording, and everything else
    /// reaches the frontend. `accept-edits` adds exactly one pre-answer of its
    /// own — the workspace file token — and it is not a grant: the call is
    /// neither recorded as granted nor counted. The pre-answer is refused
    /// under the freeze, before it is consulted, because a mode's consent is
    /// a reader's.
    fn asking_arm(&self, grant_token: Option<&str>) -> ApprovalDecision {
        let pre_answered = !self.frozen
            && self.mode() == ApprovalPolicy::AcceptEdits
            && grant_token == Some(WORKSPACE_WRITE_TOKEN);
        if pre_answered {
            return ApprovalDecision::Allow;
        }
        match grant_token {
            Some(token) if self.grants.is_granted(token) => {
                self.grants.record_allowed_call(token);
                ApprovalDecision::Allow
            }
            _ if self.frozen => ApprovalDecision::Deny {
                reason: Some(HEADLESS_ASK_REASON),
            },
            _ => ApprovalDecision::Ask,
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
