//! The live session's engine-side runtime: the composed tool universe and
//! the single-writer lock on its state directory, held for the process and
//! released at exit. Per SESSION-WORKSPACE.md, the lock rides the session
//! state (`sessions/<id>/lock`), never the user's project — two sessions on
//! one project are two writers among the editors and build tools that
//! already share the tree.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use saya_agent::{ApprovalPolicy, SessionPolicy};
use saya_harness::session_lifecycle::SessionLock;
use saya_store::{BypassSource, SessionJournal};

use crate::agent::question_scope::QuestionScope;
use crate::interactive::session::lifecycle::paths::create_state_dir;
use crate::interactive::session::lifecycle::recovery::RecoveryHandle;
use crate::interactive::session::universe::{SessionComposition, SessionUniverse};
use crate::local_metrics::{LocalMetrics, LocalPhaseSnapshot};

pub(crate) mod context_key;

pub(crate) struct SessionRuntime {
    universe: Arc<SessionUniverse>,
    /// The session's approval policy, hoisted to session lifetime: one
    /// instance per session, cloned into each turn's decider, whose clone
    /// shares the grant store. A grant recorded in one turn is in force in
    /// the next. Process lifetime only — never persisted — and a resumed
    /// session starts empty.
    policy: SessionPolicy,
    /// The mode `policy` was built with, so `sync_policy` can tell a real
    /// mode change from a no-op without reading the policy engine.
    policy_mode: ApprovalPolicy,
    /// Held, never read: the lock's purpose is its lifetime — dropping the
    /// runtime releases it.
    _lock: SessionLock,
    /// The explicit `--workspace` statement, carried so a mid-session
    /// `/resume` re-binds the same way the launch did.
    explicit: Option<PathBuf>,
    /// Whether this runtime is a fresh start: only a fresh start can answer
    /// the startup trust question, and only once — answering records the
    /// trusted directory as the launch's statement (see `bind_trusted`).
    fresh_start: bool,
    /// The session journal: `sessions/<id>/journal.ndjson`, the audit record
    /// of what the user consented to. Opened under this runtime's lock; a
    /// torn tail from a crashed process is healed at open. Never read back
    /// into the grant store — a resumed session starts empty.
    journal: Arc<SessionJournal>,
    /// Where `sessions/<id>/` lives, carried so a mid-session `/resume`
    /// claims the resumed state directory in the same root the launch did.
    sessions_root: PathBuf,
    /// The state directory this session holds (`sessions/<id>/`), carried
    /// so a fresh-start host-lane recomposition re-enters it.
    state_dir: PathBuf,
    /// A journal write that failed at the launch site, said once by the
    /// notice seam the surfaces already print. Later failures are said by
    /// the site that made the consent.
    journal_failure: Mutex<Option<String>>,
    /// Runtime-only question authority. It deliberately never rides
    /// `SessionState`, so a saved narrative cannot restore a cancellation
    /// token or answering wallet after resume.
    question: QuestionLifecycle,
    /// The last settled question's fixed local-metrics snapshot for the
    /// print-after-restore seam: runtime-only, never serialized, last only.
    last_question_metrics: Option<LocalPhaseSnapshot>,
    /// The session's local metrics collector, when local metrics are on:
    /// runtime-only, never serialized, and never in `SessionState`.
    session_metrics: Option<Arc<LocalMetrics>>,
    /// The session's recovery handle: the checkpoint store over this
    /// sessions root, the acquired id, and the session-scoped enable
    /// flag — OFF at acquisition (recovery is opt-in). One handle per
    /// acquisition: every recomposition carries the same one, and a
    /// resume re-acquires so it starts off again.
    recovery: RecoveryHandle,
}

enum QuestionLifecycle {
    Idle,
    Active(QuestionScope),
    PendingClarification(QuestionScope),
}

/// The acquisition's inputs, including the trust seam: `trusted: None` is
/// every path that did not answer the startup trust prompt. Production
/// fills it from the prompt's answer in `session_loop`; `acquire` passes
/// `None` directly.
pub(crate) struct Acquire<'a> {
    pub(crate) runtime: &'a crate::config::runtime::RuntimeConfig,
    pub(crate) explicit: Option<&'a Path>,
    pub(crate) fresh: bool,
    pub(crate) pinned_root: Option<&'a str>,
    pub(crate) id: &'a str,
    pub(crate) mode: ApprovalPolicy,
    pub(crate) sessions_root: &'a Path,
    pub(crate) trusted: Option<&'a Path>,
}

include!("acquire.rs");
include!("question_lifecycle.rs");
include!("composition.rs");
include!("accessors.rs");

/// A policy for `mode` carrying `tokens` — the one pure step of
/// [`SessionRuntime::sync_policy`], testable without acquiring a session.
fn policy_carrying(mode: ApprovalPolicy, tokens: &[String]) -> SessionPolicy {
    let policy = SessionPolicy::new(mode);
    for token in tokens {
        policy.grants().grant(token);
    }
    policy
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mid-session `/approval` swap carries the session's grants: the
    /// policy is rebuilt for the new mode, but every token the user granted
    /// stays granted — a mode excursion must not silently revoke an explicit
    /// session grant.
    #[test]
    fn a_mode_swap_carries_the_session_s_grants() {
        let policy = policy_carrying(ApprovalPolicy::Ask, &["runner:bench".to_owned()]);
        assert!(policy.grants().is_granted("runner:bench"));
        let carried = policy_carrying(ApprovalPolicy::ReadOnly, &policy.grants().tokens());
        assert!(
            carried.grants().is_granted("runner:bench"),
            "the grant survives the mode swap"
        );
        assert_eq!(
            carried.resolve(&side_effecting(), Some("runner:bench")),
            saya_agent::ApprovalDecision::Deny { reason: None },
            "read-only never asks, so the carried grant still cannot move it"
        );
        let back = policy_carrying(ApprovalPolicy::Ask, &carried.grants().tokens());
        assert_eq!(
            back.resolve(&side_effecting(), Some("runner:bench")),
            saya_agent::ApprovalDecision::Allow,
            "back under ask, the grant pre-answers as before"
        );
    }

    /// A swap into — and back out of — `bypass` carries the grants the same
    /// way. Under bypass the grant is inert (no ask consults it, nothing
    /// records through it), so the store rides the excursion unchanged: a
    /// grant made under ask is still held after the return, and pre-answers
    /// again the moment the mode is ask once more.
    #[test]
    fn a_mode_swap_into_and_out_of_bypass_carries_the_grants() {
        let policy = policy_carrying(ApprovalPolicy::Ask, &["runner:bench".to_owned()]);
        // Into bypass: the grant is held but never consulted — the mode
        // allows on its own.
        let bypassed = policy_carrying(ApprovalPolicy::Bypass, &policy.grants().tokens());
        assert!(
            bypassed.grants().is_granted("runner:bench"),
            "the grant survives the swap into bypass"
        );
        assert_eq!(
            bypassed.resolve(&side_effecting(), Some("runner:bench")),
            saya_agent::ApprovalDecision::Allow,
            "under bypass the mode allows, the grant is not the judge"
        );
        // Back out to ask: the grant pre-answers again, exactly as before the
        // excursion — nothing under bypass recorded or revoked.
        let back = policy_carrying(ApprovalPolicy::Ask, &bypassed.grants().tokens());
        assert_eq!(
            back.resolve(&side_effecting(), Some("runner:bench")),
            saya_agent::ApprovalDecision::Allow,
            "back under ask, the pre-bypass grant is intact and pre-answers"
        );
        assert_eq!(
            back.resolve(&side_effecting(), Some("runner:deploy")),
            saya_agent::ApprovalDecision::Ask,
            "a shape the user never granted still asks"
        );
    }

    fn side_effecting() -> saya_agent::ToolEffect {
        saya_agent::ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: saya_agent::LocalStateEffect::None,
        }
    }
}

#[cfg(test)]
#[path = "question_tests.rs"]
mod question_tests;

#[cfg(test)]
#[path = "recovery_acquire_tests.rs"]
mod recovery_acquire_tests;
