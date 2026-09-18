//! The live session's engine-side runtime: the composed tool universe and
//! the single-writer lock on its state directory, held for the process and
//! released at exit. Per SESSION-WORKSPACE.md, the lock rides the session
//! state (`sessions/<id>/lock`), never the user's project — two sessions on
//! one project are two writers among the editors and build tools that
//! already share the tree.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use saya_agent::{ApprovalPolicy, SessionPolicy};
use saya_harness::lock::RunLock;
use saya_store::{BypassSource, SessionJournal};

use super::session_paths::create_state_dir;
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

impl SessionRuntime {
    /// Acquires the session: the state directory (0700), the lock (refusing
    /// a live holder by pid, reclaiming a stale file), and the composed
    /// universe. `fresh` distinguishes a first start — the git worktree top
    /// binds when nothing is stated — from a resume, which re-opens the
    /// recorded pin and runs unbound when the record has none. The approval
    /// policy is built once here, from the session's mode — empty by
    /// construction: a resumed session inherits no grant, from the record
    /// or from the journal, which is the audit record, never a grant source.
    /// `sessions_root` is where `sessions/<id>/` lives, so a test can claim
    /// a root of its own; production passes [`default_session_dir`].
    pub(crate) fn acquire(
        runtime: &crate::config::runtime::RuntimeConfig,
        explicit: Option<&Path>,
        fresh: bool,
        pinned_root: Option<&str>,
        id: &str,
        mode: ApprovalPolicy,
        sessions_root: &Path,
    ) -> Result<Self, String> {
        Self::acquire_inner(Acquire {
            runtime,
            explicit,
            fresh,
            pinned_root,
            id,
            mode,
            sessions_root,
            trusted: None,
        })
    }

    /// The composed acquisition behind both paths: `acquire` (no trust
    /// answer) and the startup trust prompt's answer in `session_loop`,
    /// which fills `Acquire.trusted` directly. One struct argument keeps the
    /// arity lint's budget; the seven-argument public seam above is
    /// untouched.
    pub(crate) fn acquire_inner(args: Acquire<'_>) -> Result<Self, String> {
        let Acquire {
            runtime,
            explicit,
            fresh,
            pinned_root,
            id,
            mode,
            sessions_root,
            trusted,
        } = args;
        let state_dir = create_state_dir(sessions_root, id)?;
        let lock = RunLock::acquire(state_dir.join("lock")).map_err(|error| match error {
            // The same words a run's second writer reads, with the same pid.
            saya_harness::HarnessError::LockHeld { pid } => format!(
                "session {id} is already running (pid {pid}): a session's scratch and \
                 transcript are single-writer; stop the other process or start a new session"
            ),
            other => format!("could not claim the session lock: {other}"),
        })?;
        let walk_when_unpinned = fresh;
        // The trusted directory binds exactly like an explicit
        // `--workspace` — but only where no explicit statement exists, and
        // only on a fresh start: a resume re-opens its recorded pin and never
        // re-prompts (G3: re-trust is per-process, never persisted).
        let trusted_explicit = match (fresh, explicit, trusted) {
            (true, None, Some(dir)) => Some(dir.to_path_buf()),
            _ => None,
        };
        let asked = trusted_explicit.clone();
        let effective_explicit: Option<&Path> =
            explicit.or(trusted_explicit.as_deref().map(Path::new));
        let universe = SessionUniverse::compose(
            runtime,
            effective_explicit,
            pinned_root,
            walk_when_unpinned,
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            &state_dir,
        )?;
        let explicit = explicit.map(Path::to_path_buf).or(asked);
        Ok(Self {
            universe: Arc::new(universe),
            policy: SessionPolicy::new(mode),
            policy_mode: mode,
            _lock: lock,
            explicit,
            fresh_start: fresh,
            sessions_root: sessions_root.to_path_buf(),
            state_dir: state_dir.clone(),
            journal: Arc::new(SessionJournal::open(&state_dir)),
            journal_failure: Mutex::new(None),
        })
    }

    /// Swaps the composed universe for a startup trust answer given
    /// inside the TUI: recomposes with the trusted directory bound exactly
    /// like an explicit `--workspace`. Only on a fresh start with no
    /// explicit statement — the same rule `acquire_inner` applies — so a
    /// resume or an explicit bind can never reach here.
    pub(crate) fn bind_trusted(
        &mut self,
        runtime: &crate::config::runtime::RuntimeConfig,
        trusted: &Path,
    ) -> Result<(), String> {
        if !self.fresh_start() || self.explicit_statement().is_some() {
            return Err(
                "the trust answer applies only to a fresh start with no --workspace".into(),
            );
        }
        let universe = SessionUniverse::compose(
            runtime,
            Some(trusted),
            None,
            true,
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            &self.state_dir,
        )?;
        self.explicit = Some(trusted.to_path_buf());
        self.universe = Arc::new(universe);
        Ok(())
    }

    /// Whether this runtime is a fresh start: only a fresh start can answer
    /// the startup trust question, and only once — answering records the
    /// trusted directory as the launch's statement (see `bind_trusted`).
    pub(crate) fn fresh_start(&self) -> bool {
        self.fresh_start
    }

    /// The launch's explicit `--workspace` statement, when one exists: the
    /// trust answer binds only where this is `None`.
    pub(crate) fn explicit_statement(&self) -> Option<&Path> {
        self.explicit.as_deref()
    }

    /// The session's journal handle — what `/allow`, the deciders, and the
    /// bypass activation sites write through. Clones share the file.
    pub(crate) fn journal(&self) -> Arc<SessionJournal> {
        Arc::clone(&self.journal)
    }

    /// The state directory this session holds (`sessions/<id>/`) — what a
    /// fresh-start recomposition re-enters.
    pub(crate) fn state_dir(&self) -> PathBuf {
        self.state_dir.clone()
    }

    /// Swaps the composed universe: the fresh-start host-lane recomposition.
    /// The lock, policy, journal, and pins ride the swap untouched.
    pub(crate) fn replace_universe(&mut self, universe: super::session_universe::SessionUniverse) {
        self.universe = Arc::new(universe);
    }

    /// Journals one bypass activation and records a failure for the notice
    /// seam — the launch site has no message of its own to say it in, so
    /// the startup notice the surfaces already print carries it. The first
    /// failure is kept; a journal that cannot write will keep failing.
    pub(crate) fn journal_bypass_activation(&self, source: BypassSource) {
        if let Err(error) = self.journal.bypass_activated(source) {
            let mut failure = self
                .journal_failure
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if failure.is_none() {
                *failure = Some(super::session_grants::journal_warning(&error));
            }
        }
    }

    /// The startup notice the universe reported, if any — a vanished pin
    /// must be said, never silent — plus a journal write that failed at the
    /// launch site, if one did.
    pub(crate) fn notice(&self) -> Option<String> {
        let universe = self.universe.notice.as_deref();
        let journal = self
            .journal_failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        match (universe, journal) {
            (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
            (Some(a), None) => Some(a.to_owned()),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
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
            &self.sessions_root,
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
