impl SessionRuntime {
    /// Swaps the composed universe for a startup trust answer given
    /// inside the TUI: recomposes with the trusted directory bound exactly
    /// like an explicit `--workspace`. Only on a fresh start with no
    /// explicit statement — the same rule `acquire_inner` applies — so a
    /// resume or an explicit bind can never reach here.
    pub(crate) fn bind_trusted(
        &mut self,
        runtime: &crate::config::runtime::RuntimeConfig,
        trusted: &Path,
        launch: Option<&crate::interactive::session_host::HostLaunch>,
    ) -> Result<(), String> {
        if !self.fresh_start() || self.explicit_statement().is_some() {
            return Err(
                "the trust answer applies only to a fresh start with no --workspace".into(),
            );
        }
        let universe = self.compose_universe(runtime, Some(trusted), None, true, launch)?;
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

    /// Recompose after the launch statement is available. The lock, policy,
    /// journal, pins, and live scratch connection ride the swap untouched.
    pub(crate) fn recompose_with_launch(
        &mut self,
        runtime: &crate::config::runtime::RuntimeConfig,
        pinned_root: Option<&str>,
        launch: Option<&crate::interactive::session_host::HostLaunch>,
    ) -> Result<(), String> {
        let universe = self.compose_universe(
            runtime,
            self.explicit_statement(),
            pinned_root,
            self.fresh_start,
            launch,
        )?;
        self.universe = Arc::new(universe);
        Ok(())
    }

    fn compose_universe(
        &self,
        runtime: &crate::config::runtime::RuntimeConfig,
        explicit: Option<&Path>,
        pinned_root: Option<&str>,
        walk_when_unpinned: bool,
        launch: Option<&crate::interactive::session_host::HostLaunch>,
    ) -> Result<SessionUniverse, String> {
        SessionUniverse::compose_with_launch_and_path(SessionComposition {
            runtime,
            explicit,
            pinned_root,
            walk_when_unpinned,
            cwd: &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            state_dir: &self.state_dir,
            launch,
            path: std::env::var_os("PATH").map(|value| value.to_string_lossy().into_owned()),
            scratch: self.universe.scratch(),
            recovery: Some(self.recovery().clone()),
        })
    }
}
