impl SessionRuntime {
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
                *failure =
                    Some(crate::interactive::session::permissions::grants::journal_warning(&error));
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

    /// The session's recovery handle: what the per-turn attach reads and
    /// the toggle will flip. One handle per acquisition; clones share the
    /// flag cell.
    pub(crate) fn recovery(&self) -> &RecoveryHandle {
        &self.recovery
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
        // Only reached when the new session is fully held and composed, so
        // failure is impossible past this point: the swap first abandons the
        // old session's question — the same effect `abandon_question` has —
        // and a failed swap above returned early, leaving it untouched. The
        // swap releases the old lock after the replacement is held.
        self.abandon_question();
        *self = replacement;
        Ok(())
    }
}
