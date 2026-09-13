//! The live session's engine-side runtime: the composed tool universe and
//! the single-writer lock on its state directory, held for the process and
//! released at exit. Per SESSION-WORKSPACE.md, the lock rides the session
//! state (`sessions/<id>/lock`), never the user's project — two sessions on
//! one project are two writers among the editors and build tools that
//! already share the tree.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_agent::{ApprovalPolicy, SessionPolicy};
use saya_harness::lock::RunLock;

use super::session_paths::{create_state_dir, default_session_dir};
use super::session_universe::SessionUniverse;

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
    _lock: RunLock,
    /// The explicit `--workspace` statement, carried so a mid-session
    /// `/resume` re-binds the same way the launch did.
    explicit: Option<PathBuf>,
}

impl SessionRuntime {
    /// Acquires the session: the state directory (0700), the lock (refusing
    /// a live holder by pid, reclaiming a stale file), and the composed
    /// universe. `fresh` distinguishes a first start — the git worktree top
    /// binds when nothing is stated — from a resume, which re-opens the
    /// recorded pin and runs unbound when the record has none. The approval
    /// policy is built once here, from the session's mode.
    pub(crate) fn acquire(
        runtime: &crate::config::runtime::RuntimeConfig,
        explicit: Option<&Path>,
        fresh: bool,
        pinned_root: Option<&str>,
        id: &str,
        mode: ApprovalPolicy,
    ) -> Result<Self, String> {
        let state_dir = create_state_dir(&default_session_dir(), id)?;
        let lock = RunLock::acquire(state_dir.join("lock")).map_err(|error| match error {
            // The same words a run's second writer reads, with the same pid.
            saya_harness::HarnessError::LockHeld { pid } => format!(
                "session {id} is already running (pid {pid}): a session's scratch and \
                 transcript are single-writer; stop the other process or start a new session"
            ),
            other => format!("could not claim the session lock: {other}"),
        })?;
        let walk_when_unpinned = fresh;
        let universe = SessionUniverse::compose(
            runtime,
            explicit.map(Path::new),
            pinned_root,
            walk_when_unpinned,
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            &state_dir,
        )?;
        let explicit = explicit.map(Path::to_path_buf);
        Ok(Self {
            universe: Arc::new(universe),
            policy: SessionPolicy::new(mode),
            policy_mode: mode,
            _lock: lock,
            explicit,
        })
    }

    /// The session's approval policy, cloned into a turn's decider. A clone
    /// shares the grant store (`SessionGrants` is `Arc<Mutex<_>>`-backed), so
    /// grants recorded through any turn's ask are visible to every other.
    pub(crate) fn policy(&self) -> SessionPolicy {
        self.policy.clone()
    }

    /// Points the session's policy at the session's current approval mode —
    /// what a mid-session `/approval` needs. The grants the user made this
    /// session ride the swap: they are explicit, additive facts about the
    /// session, and no mode consults them that would not have before
    /// (read-only and never never ask, so grants cannot move them — the
    /// engine's own rule). A rebuilt-empty policy would silently revoke them.
    pub(crate) fn sync_policy(&mut self, mode: ApprovalPolicy) {
        if self.policy_mode == mode {
            return;
        }
        self.policy = policy_carrying(mode, &self.policy.grants().tokens());
        self.policy_mode = mode;
    }

    /// The composed universe, shared with every turn of this session.
    pub(crate) fn universe(&self) -> Arc<SessionUniverse> {
        Arc::clone(&self.universe)
    }

    /// The canonical workspace root, when one binds.
    pub(crate) fn root(&self) -> Option<&Path> {
        self.universe.root()
    }

    /// The pinned root the session record should carry: the resolved root
    /// when this process bound one explicitly or freshly; `None` (keep the
    /// record's existing pin) when a resume re-opened a recorded root that
    /// has since vanished.
    pub(crate) fn record_root(&self, fresh: bool) -> Option<String> {
        if fresh || self.explicit.is_some() {
            return self.root().map(|root| root.display().to_string());
        }
        None
    }

    /// The startup notice the universe reported, if any — a vanished pin
    /// must be said, never silent.
    pub(crate) fn notice(&self) -> Option<&str> {
        self.universe.notice.as_deref()
    }

    /// Swaps the runtime to a resumed session id: the new state directory is
    /// claimed (refusing a live holder) and composed before the old lock
    /// releases, so a failed swap leaves the current session still held. The
    /// resumed session's policy is its own, built from the resumed mode —
    /// grants are process-lifetime facts about one session, and a resumed
    /// session starts empty.
    pub(crate) fn reacquire(
        &mut self,
        runtime: &crate::config::runtime::RuntimeConfig,
        pinned_root: Option<&str>,
        id: &str,
        mode: ApprovalPolicy,
    ) -> Result<(), String> {
        let replacement = Self::acquire(
            runtime,
            self.explicit.as_deref(),
            false,
            pinned_root,
            id,
            mode,
        )?;
        // Only reached when the new session is fully held and composed; the
        // swap drops this lock last.
        *self = replacement;
        Ok(())
    }
}

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
