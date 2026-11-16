struct RootGuard(PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One fixture for the real TUI dispatch: the exact pieces the live loop
/// passes (`loop_tick::pending`'s shape), backed by a throwaway root.
struct TuiHarness {
    app: App,
    state: SessionState,
    session: SessionRuntime,
    store: FsSessionStore,
    runtime: RuntimeConfig,
    /// Removes the throwaway root when the fixture drops (the chain-test
    /// precedent): declared after the session runtime so the lock releases
    /// before the directory goes.
    ///
    /// Held, never read: the guard's purpose is its lifetime — dropping the
    /// fixture removes the throwaway root (the `_lock` precedent).
    _root: RootGuard,
}

impl TuiHarness {
    fn build(tag: &str) -> Self {
        let root = temp_sessions_root(tag);
        let mut runtime = unused_runtime();
        // The composed investigations root: anything the dispatch runs
        // writes under this fixture's own root, never the machine's data
        // directory.
        runtime.investigations_root = root.join("investigations");
        let session = SessionRuntime::acquire(
            &runtime,
            Some(&root),
            true,
            None,
            "context-key-tui",
            ApprovalPolicy::Ask,
            &root.join("sessions"),
        )
        .expect("the TUI test session acquires");
        Self {
            app: idle_app(),
            state: SessionState::new("context-key-tui", None, "m1"),
            session,
            store: FsSessionStore::new(root.join("session-store")),
            runtime,
            _root: RootGuard(root),
        }
    }

    /// Normalizes the lifecycle to PendingClarification, the same way the
    /// table's harness does.
    fn pend(&mut self, label: &str) -> QuestionScope {
        self.session.abandon_question();
        let scope = self
            .session
            .begin_question(&self.runtime)
            .unwrap_or_else(|error| panic!("{label}: the question begins: {error}"));
        self.session.settle_question(&scope, true);
        scope
    }

    /// The real `dispatch`, called with the fixture's pieces exactly as the
    /// live loop does.
    fn dispatch_line(&mut self, line: &str) {
        dispatch(
            line,
            &mut self.app.transcript,
            &self.app.profiles,
            &mut self.state,
            &self.runtime,
            &self.store,
            &self.app.state_db,
            RenderFormat::Text,
            &mut self.app.last_query,
            &self.app.captured,
            self.app.agent_captures.gap,
            &mut self.session,
        );
    }

    fn last_system_block(&self) -> Option<String> {
        self.app
            .transcript
            .blocks()
            .iter()
            .rev()
            .find(|block| block.kind == BlockKind::System)
            .map(|block| block.text.clone())
    }
}
