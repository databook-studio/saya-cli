impl SessionUniverse {
    /// Whether the host lane composed — the test seam the red tests read.
    #[cfg(test)]
    pub(crate) fn host_composed_for_tests(&self) -> Option<()> {
        self.host.as_ref().map(|_| ())
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
            runner: self.runner.as_ref().map(|runner| runner.prompt_facts()),
            fetch: self
                .fetch
                .as_ref()
                .map(|fetch| crate::approval_facts::FetchFacts {
                    fetch_body_bytes: lane.max_total_bytes,
                    fetch_seconds: lane.time_budget.as_secs(),
                    fetch_redirects: lane.max_redirect_hops,
                    download: Some(fetch.download_budget().clone()),
                }),
            scratch: self.scratch.as_ref().map(|_| {
                // The session's scratch runs under the harness's own
                // constants (opened in `compose` without narrowing).
                crate::approval_facts::ScratchFacts {
                    row_cap: saya_harness::scratch::SCRATCH_ROW_CAP,
                    timeout_seconds: saya_harness::scratch::SCRATCH_QUERY_TIMEOUT.as_secs(),
                }
            }),
            workspace_root: self.root().map(|root| root.to_path_buf()),
            host: self.host.as_ref().map(|host| host.facts.clone()),
            host_ran: self.host_ran.load(std::sync::atomic::Ordering::SeqCst),
            denied_programs: self.deny.programs(),
        }
    }

    /// The containment seam the file tools read and write through, when a
    /// root binds. `None` leaves `workspace_read` denying with its typed
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

    /// The live scratch handle for a recomposition. Cloning it keeps the
    /// DuckDB connection alive while the next universe rebinds its workspace.
    pub(crate) fn scratch(&self) -> Option<Arc<ScratchSql>> {
        self.scratch.clone()
    }

    /// The deny list this universe carries — the programs every door
    /// refuses. Read by the session loop's start-event journal site.
    pub(crate) fn deny_programs(&self) -> Vec<String> {
        self.deny.programs()
    }

    /// The session's live task list — what `tasks_set` writes, what each
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

    /// The shared host-ran flag — the executor's host member sets it after a
    /// host call settles. Cloned into `RunTools` once per executor.
    pub(crate) fn host_ran_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.host_ran)
    }
}
