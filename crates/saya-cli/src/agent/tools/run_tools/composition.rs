impl RunTools {
    /// Composes the members into the one executor. Every surface that
    /// advertises a harness tool dispatches through this constructor — there
    /// is no second assembly path.
    pub(crate) fn compose(database: Arc<DatabaseTools>, fetch: Option<Arc<FetchTools>>) -> Self {
        Self {
            database,
            fetch,
            host: None,
            deny: crate::interactive::session::permissions::deny::SessionDeny::default(),
            tasks: None,
            journal: None,
            tools: saya_config::ResolvedTools::default(),
        }
    }

    /// Narrows this composite to the `[tools] allow` list its surface
    /// resolved. The list only ever removes members: a name it holds that the
    /// surface never composed stays refused, exactly as before.
    pub(crate) fn with_tools(mut self, tools: saya_config::ResolvedTools) -> Self {
        self.tools = tools;
        self
    }

    /// Whether this surface may dispatch `name` at all. The same predicate
    /// [`super::narrow`] filtered the advertisement by; the shell wrappers
    /// around this composite ask it before routing a `shell` call, which they
    /// serve themselves.
    pub(crate) fn admits(&self, name: &str) -> bool {
        super::admits(&self.tools, name)
    }

    /// Composes with the session deny list: refused at the `run_command`
    /// door before anything else. The deleted session executor was its
    /// production caller; the `run_command` reach tests bind the list here so
    /// the deny matcher's branch is exercised rather than assumed.
    #[cfg(test)]
    pub(crate) fn with_session_deny(
        mut self,
        deny: crate::interactive::session::permissions::deny::SessionDeny,
    ) -> Self {
        self.deny = deny;
        self
    }

    /// Composes with the session journal: a deny firing is written there
    /// before the refusal is relayed. Wired by `SessionUniverse`'s
    /// journal-carrying executor; the deny tests attach it directly.
    #[cfg(test)]
    pub(crate) fn with_session_journal(
        mut self,
        journal: std::sync::Arc<saya_store::SessionJournal>,
    ) -> Self {
        self.journal = Some(journal);
        self
    }

    /// Composes with the host lane: the executor config, whose workspace
    /// root the child runs with as its cwd. The session surface is the only
    /// caller — runs never get the lane. The `run_command` tests compose it
    /// here now that the session executor is gone.
    #[cfg(test)]
    pub(crate) fn with_host(
        mut self,
        config: saya_harness::host::HostConfig,
        cancellation: &CancellationToken,
    ) -> Self {
        self.host = Some(HostCommandMember {
            config,
            cancellation: cancellation.clone(),
        });
        self
    }
}
