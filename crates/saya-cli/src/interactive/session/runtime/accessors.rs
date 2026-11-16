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
    /// The task posture rides it too: a consent-mode swap is not a change of
    /// what the task may touch, and dropping it would quietly widen a `plan`
    /// session's write surface at the engine. The plan gate rides it for the
    /// same reason: the rebuilt policy holds the composed gate's own cell, so
    /// an approval already given still admits what the advertisement lists.
    pub(crate) fn sync_policy(&mut self, mode: ApprovalPolicy) {
        if self.policy_mode == mode {
            return;
        }
        self.policy = policy_carrying(
            mode,
            self.policy.agent_mode(),
            &self.policy.grants().tokens(),
        )
        .with_plan_gate(Some(self.universe.plan_gate().cell()));
        self.policy_mode = mode;
    }

    /// Applies a permission-mode switch to the session's one policy, in
    /// place: the shared mode cells move, so a clone already handed to an
    /// in-flight turn's decider reads the new posture at its next `resolve`,
    /// and the bookkeeping mode follows, so the next turn's [`sync_policy`]
    /// is a no-op rather than a rebuild. `None` leaves the consent posture
    /// alone — a `plan` switch is a task posture only. Reports whether the
    /// policy accepted the switch; a frozen policy (the headless run's)
    /// refuses it, and a refusal changes nothing.
    ///
    /// [`sync_policy`]: Self::sync_policy
    pub(crate) fn switch_policy(
        &mut self,
        mode: Option<ApprovalPolicy>,
        agent_mode: AgentMode,
    ) -> bool {
        let Some(mode) = mode else {
            return self.policy.set_agent_mode(agent_mode);
        };
        if !self.policy.set_mode(mode) {
            return false;
        }
        self.policy_mode = mode;
        self.policy.set_agent_mode(agent_mode)
    }

    /// The composed universe, shared with every turn of this session.
    pub(crate) fn universe(&self) -> Arc<SessionUniverse> {
        Arc::clone(&self.universe)
    }

    /// Runs the `SessionStart` hooks once, before the first turn. The bounded
    /// stdout is kept on the universe and offered to the model by every turn's
    /// hook gate. A session with no root, no hook configured, or a mode with
    /// no host lane runs none.
    pub(crate) fn start_hooks(&self, runtime: &crate::config::runtime::RuntimeConfig) {
        let Some(hooks) = self.session_hooks(runtime) else {
            return;
        };
        let cell = self.universe.session_hooks();
        crate::interactive::session::lifecycle::resume::block_on(async {
            cell.start(&hooks).await
        });
    }

    /// Runs the `SessionEnd` hooks: best-effort, on the normal end and after a
    /// cancel alike, each still bounded by its own timeout.
    pub(crate) fn end_hooks(&self, runtime: &crate::config::runtime::RuntimeConfig) {
        if let Some(hooks) = self.session_hooks(runtime) {
            crate::interactive::session::lifecycle::resume::block_on(hooks.session_end());
        }
    }

    fn session_hooks(
        &self,
        runtime: &crate::config::runtime::RuntimeConfig,
    ) -> Option<Arc<crate::agent::hooks::Hooks>> {
        // The session's root when one binds; otherwise the directory the
        // process was started in, exactly where a turn's hooks run.
        let root = self
            .root()
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())?;
        crate::agent::hooks::Hooks::compose(
            runtime,
            self.policy_mode,
            &root,
            crate::agent::hooks::HookCaller::Session(self.universe.session_id()),
            saya_agent::CancellationToken::new(),
        )
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
