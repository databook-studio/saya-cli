impl SessionUniverse {
    /// Whether the host lane composed — the test seam the red tests read.
    #[cfg(test)]
    pub(crate) fn host_composed_for_tests(&self) -> Option<()> {
        self.host.as_ref().map(|_| ())
    }

    /// This session's started MCP servers, or `None` where no server is
    /// configured or the mode reaches none. Carried across a recomposition,
    /// so a recomposed session keeps the servers it already started.
    pub(crate) fn mcp(&self) -> Option<Arc<McpTools>> {
        self.mcp.clone()
    }

    /// This session's MCP servers under one turn's consent: the mode, the
    /// decider that is here now, and this turn's cancellation. `None` where
    /// no lane was composed or no server started; a lane whose servers all
    /// failed still reports why in the startup notice.
    pub(crate) fn mcp_for_turn(
        &self,
        policy: ApprovalPolicy,
        consent: crate::mcp::client::Consent,
        cancellation: &CancellationToken,
    ) -> Option<Arc<McpTools>> {
        let lane = self.mcp.as_ref()?;
        (!lane.is_empty()).then(|| {
            lane.with_consent(
                policy,
                consent,
                crate::agent::tools::consent::Caller::Session,
                cancellation.clone(),
            )
        })
    }

    /// Whether the host lane composed: the bypass activation line's lane
    /// fact and the status header's `host:` segment read this.
    pub(crate) fn host_composed(&self) -> bool {
        self.host.is_some()
    }

    /// The canonical workspace root, when one binds — the status header's
    /// and the session record's fact.
    pub(crate) fn root(&self) -> Option<&Path> {
        self.workspace.as_ref().map(|bound| bound.root.as_path())
    }

    /// The checkout's data-file route, detected once at composition, gated
    /// on whether this turn advertises `shell`.
    pub(crate) fn file_sources(
        &self,
        shell_offered: bool,
    ) -> crate::agent::file_sources::FileSources {
        crate::agent::file_sources::for_session(shell_offered, self.file_sources)
    }

    /// This session's record id, when a live acquisition composed this
    /// universe — the identity a hook's input carries as `session_id`. A
    /// universe with no acquisition behind it (a wrapper, a test fixture)
    /// has none, and a hook then reads no id rather than a made-up one.
    pub(crate) fn session_id(&self) -> Option<String> {
        self.recovery
            .as_ref()
            .map(crate::interactive::session::lifecycle::recovery::RecoveryHandle::session_id)
    }

    /// The `SessionStart` context cell: run once at launch, read by every
    /// turn's hook gate. Shared across a recomposition.
    pub(crate) fn session_hooks(&self) -> Arc<crate::agent::hooks::SessionHooks> {
        Arc::clone(&self.session_hooks)
    }

    /// Keeps the `SessionStart` cell a recomposition already used: a
    /// recomposition rebinds the workspace, and the hook runs once per
    /// session process.
    pub(crate) fn carry_session_hooks(
        &mut self,
        previous: Arc<crate::agent::hooks::SessionHooks>,
    ) {
        self.session_hooks = previous;
    }

    /// This session's post-turn extraction circuit breaker — the runtime
    /// consults it before every turn's extraction attempt and records this
    /// turn's outcome against it. Not persisted: a resumed session gets a
    /// fresh one, composed enabled.
    pub(crate) fn learning_breaker(&self) -> &LearningBreaker {
        &self.learning_breaker
    }

    /// Binds the turn's live grant store — what the session surfaces do
    /// once per turn, from the policy they hold (`session_request::run`,
    /// the TUI spawn). Mirrors `primary`'s per-turn bind; an unbound view
    /// is the closed shape.
    pub(crate) fn bind_grants(&self, grants: &saya_agent::SessionGrants) {
        self.grants.bind(grants);
    }

    /// The approval prompts' facts, read off this universe's composed
    /// members and the resolved config. The prompt states what these
    /// enforce and nothing else: a member that was not composed contributes
    /// no fact lines, and every stated number is the enforcement's own.
    /// The deny list rides along: it gates every program-named door even
    /// with the host lane off, so the deciders read it from here too.
    pub(crate) fn approval_facts(
        &self,
        runtime: &crate::config::runtime::RuntimeConfig,
    ) -> crate::approval_facts::ApprovalFacts {
        // The fetch member was composed with exactly these lane bounds
        // (`compose`: `FetchLimits::for_tool_lane()`), and the same
        // constructor here is the same numbers by construction.
        let lane = saya_harness::fetch::FetchLimits::for_tool_lane();
        crate::approval_facts::ApprovalFacts {
            row_cap: crate::agent::state_tools::model_row_cap(runtime.resolved.max_rows),
            sql_timeout_seconds: runtime.resolved.query_timeout_seconds,
            // The session composes no runner or interpreter door, so no prompt
            // states a runner fact.
            fetch: self
                .fetch
                .as_ref()
                .map(|fetch| crate::approval_facts::FetchFacts {
                    fetch_body_bytes: lane.max_total_bytes,
                    fetch_seconds: lane.time_budget.as_secs(),
                    fetch_redirects: lane.max_redirect_hops,
                    download: Some(fetch.download_budget().clone()),
                }),
            workspace_root: self.root().map(|root| root.to_path_buf()),
            host: self.host.as_ref().map(|host| host.facts.clone()),
            denied_programs: self.deny.programs(),
            // The deciders bind this from the journal they hold; the
            // composition itself has no session state directory.
            approval_dir: None,
        }
    }

    /// The containment seam the file tools read and write through, when a
    /// root binds. `None` leaves `read` denying with its typed
    /// error — the no-root shape.
    pub(crate) fn workspace(&self) -> Option<Arc<saya_harness::workspace::Workspace>> {
        self.workspace
            .as_ref()
            .map(|bound| Arc::clone(&bound.workspace))
    }

    /// The session's recovery handle — but only where a workspace is
    /// bound: no workspace, no file tools, no recorder, whatever the flag
    /// says. The per-turn `DatabaseTools` build attaches its parts.
    pub(crate) fn recovery_handle(&self) -> Option<RecoveryHandle> {
        if self.workspace.is_some() {
            self.recovery.clone()
        } else {
            None
        }
    }

    /// The deny list this universe carries — the programs every door
    /// refuses. Read by the session loop's start-event journal site.
    pub(crate) fn deny_programs(&self) -> Vec<String> {
        self.deny.programs()
    }

    /// The session's live task list — what `checklist` writes, what each
    /// turn renders, what the record sync reads back.
    pub(crate) fn tasks(&self) -> crate::interactive::session::tasks::SessionTasks {
        self.tasks.clone()
    }

    /// Seeds the live task list from the session record: called once per
    /// session process, after composition, with what the record carried —
    /// so a resumed session's list is what the model last wrote.
    pub(crate) fn seed_tasks(&self, list: saya_types::SessionTaskList) {
        self.tasks.replace(list);
    }
}
