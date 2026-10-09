impl SessionUniverse {
    /// The executor: the shared `RunTools` composite over this session's
    /// database tools and members, carrying the session deny list. The
    /// runner member is composed per call onto the proven spawn, carrying
    /// the turn's cancellation.
    pub(crate) fn executor(
        &self,
        database: Arc<DatabaseTools>,
        cancellation: &CancellationToken,
        permit_chart_save: bool,
    ) -> Arc<dyn ToolExecutor> {
        self.executor_with_journal(database, cancellation, None, permit_chart_save)
    }

    /// The executor with the session journal attached: a deny firing is
    /// journalled there before the refusal is relayed. The session runtime
    /// is the caller; the journal rides the turn, not the universe.
    pub(crate) fn executor_with_journal(
        &self,
        database: Arc<DatabaseTools>,
        cancellation: &CancellationToken,
        journal: Option<std::sync::Arc<saya_store::SessionJournal>>,
        permit_chart_save: bool,
    ) -> Arc<dyn ToolExecutor> {
        let runner = self.runner.as_ref().map(|runner| {
            let resolver: SharedCredentialSource =
                Arc::new(StaticCredentialSource::new(Vec::new()));
            Arc::new(
                RunProgram::for_step(
                    runner.spawn.clone(),
                    Some(runner.scope.clone()),
                    runner.interpreters.clone(),
                    runner.timeout,
                    resolver,
                )
                .with_record_dir(runner.record_dir.clone())
                .with_cancellation(cancellation.clone()),
            )
        });
        let mut tools =
            RunTools::compose(database, self.scratch.clone(), self.fetch.clone(), runner)
                .with_chart_save_permit(permit_chart_save && self.workspace.is_some())
                .with_session_deny(self.deny.clone())
                .with_tasks(self.tasks.clone());
        if let Some(journal) = journal {
            tools = tools.with_session_journal(journal);
        }
        let tools = match self.host.as_ref() {
            Some(host) => tools
                .with_host(host.config.clone(), cancellation)
                .with_host_ran(self.host_ran_flag()),
            None => tools,
        };
        Arc::new(tools)
    }
}
