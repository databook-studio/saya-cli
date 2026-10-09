impl RunTools {
    /// Composes the members into the one executor. Every surface that
    /// advertises a harness tool dispatches through this constructor — there
    /// is no second assembly path.
    pub(crate) fn compose(
        database: Arc<DatabaseTools>,
        scratch: Option<Arc<ScratchSql>>,
        fetch: Option<Arc<FetchTools>>,
        runner: Option<Arc<RunProgram>>,
    ) -> Self {
        Self {
            database,
            chart_save_permit: false,
            scratch,
            fetch,
            runner,
            host: None,
            deny: crate::interactive::session::permissions::deny::SessionDeny::default(),
            tasks: None,
            journal: None,
        }
    }

    /// Grants this composite permission to save charts into its workspace.
    pub(crate) fn with_chart_save_permit(mut self, permit: bool) -> Self {
        self.chart_save_permit = permit;
        self
    }

    /// Composes with the session deny list: refused at the `run_command`,
    /// `run_program`, and interpreter doors before anything else. The
    /// session surface is the only caller — runs never get the list.
    pub(crate) fn with_session_deny(
        mut self,
        deny: crate::interactive::session::permissions::deny::SessionDeny,
    ) -> Self {
        self.deny = deny;
        self
    }

    /// Composes with the session's live task list: `tasks_set` writes
    /// through it. The session surface is the only caller — runs never get
    /// the cell, and refuse the name as unknown.
    pub(crate) fn with_tasks(
        mut self,
        tasks: crate::interactive::session::tasks::SessionTasks,
    ) -> Self {
        self.tasks = Some(tasks);
        self
    }

    /// Composes with the session journal: a deny firing is written there
    /// before the refusal is relayed. Wired by `SessionUniverse`'s
    /// journal-carrying executor; the deny tests attach it directly.
    pub(crate) fn with_session_journal(
        mut self,
        journal: std::sync::Arc<saya_store::SessionJournal>,
    ) -> Self {
        self.journal = Some(journal);
        self
    }

    /// Composes with the host lane: the executor config, whose workspace
    /// root the child runs with as its cwd. The session surface is the only
    /// caller — runs never get the lane.
    pub(crate) fn with_host(
        mut self,
        config: saya_harness::host::HostConfig,
        cancellation: &CancellationToken,
    ) -> Self {
        self.host = Some(HostCommandMember {
            config,
            cancellation: cancellation.clone(),
            host_ran: None,
        });
        self
    }

    /// Shares the session's host-ran flag with the host member: set after a
    /// host call settles, read by later `run_program` prompts (§3 rule 6).
    /// A no-op without the lane — the flag rides the member, not the
    /// composite.
    pub(crate) fn with_host_ran(
        mut self,
        host_ran: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        if let Some(host) = self.host.as_mut() {
            host.host_ran = Some(host_ran);
        }
        self
    }
}
