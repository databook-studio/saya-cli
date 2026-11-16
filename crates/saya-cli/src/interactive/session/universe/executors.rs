use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutionContext};

impl SessionUniverse {
    /// The executor: the shared `RunTools` composite over this session's
    /// database tools and members, carrying the session deny list, with this
    /// turn's `shell` beside it.
    pub(crate) fn executor(
        &self,
        database: Arc<DatabaseTools>,
        cancellation: &CancellationToken,
        shell: Option<Arc<ShellTool>>,
        mcp: Option<Arc<McpTools>>,
        plan: Option<Arc<PlanGate>>,
        tools: saya_config::ResolvedTools,
    ) -> Arc<dyn ToolExecutor> {
        self.executor_with_journal(
            database,
            cancellation,
            None,
            TurnLanes { shell, mcp, plan },
            tools,
        )
    }

    /// The executor with the session journal attached: a deny firing is
    /// journalled there before the refusal is relayed. The session runtime
    /// is the caller; the journal rides the turn, not the universe.
    pub(crate) fn executor_with_journal(
        &self,
        database: Arc<DatabaseTools>,
        cancellation: &CancellationToken,
        journal: Option<std::sync::Arc<saya_store::SessionJournal>>,
        lanes: TurnLanes,
        tools: saya_config::ResolvedTools,
    ) -> Arc<dyn ToolExecutor> {
        let TurnLanes { shell, mcp, plan } = lanes;
        let mut tools = RunTools::compose(database, self.fetch.clone())
            .with_session_deny(self.deny.clone())
            .with_tools(tools)
            .with_tasks(self.tasks.clone());
        if let Some(journal) = journal {
            tools = tools.with_session_journal(journal);
        }
        let tools = match self.host.as_ref() {
            Some(host) => tools.with_host(host.config.clone(), cancellation),
            None => tools,
        };
        Arc::new(SessionTools {
            shell,
            background_started: Arc::clone(&self.background_started),
            mcp,
            plan,
            inner: Arc::new(tools),
        })
    }
}

/// The turn's three optional lanes — the `shell`, the MCP servers and the plan
/// gate — bundled because they travel together and are all routed by name.
/// Each is `None` where the turn composes none: no shell tool, no server
/// started, or a posture whose advertisement carries no `submit_plan`.
pub(crate) struct TurnLanes {
    shell: Option<Arc<ShellTool>>,
    mcp: Option<Arc<McpTools>>,
    plan: Option<Arc<PlanGate>>,
}

/// The session's composite: the shared `RunTools` with this turn's `shell`,
/// MCP lane and plan gate by name. The wrapper exists so the session's command
/// door, its MCP servers and its plan gate ride the same composition the coding
/// root and a run dispatch through, without teaching the shared composite —
/// which a run also holds — about a tool a session alone advertises. The
/// shell's own lanes, the MCP lane's own consent rule and the gate's own ask
/// carry their bounds: this wrapper adds no second decision.
struct SessionTools {
    shell: Option<Arc<ShellTool>>,
    /// The session's background-start latch, shared with every turn's
    /// executor: the two readers are admitted only once a run exists.
    background_started: Arc<std::sync::atomic::AtomicBool>,
    mcp: Option<Arc<McpTools>>,
    /// The plan gate, under the plan posture only: the turn passes it exactly
    /// where `submit_plan` is advertised, so the door is never wider than the
    /// list.
    plan: Option<Arc<PlanGate>>,
    inner: Arc<RunTools>,
}

impl SessionTools {
    /// The member that serves `name`, or `None` for the shared fall-through.
    fn shell_for(&self, name: &str) -> Option<&Arc<ShellTool>> {
        self.shell
            .as_ref()
            .filter(|_| shell_policy::operation(name).is_some())
    }

    /// The MCP lane, when it owns `name`: every `mcp__<server>__<tool>` call
    /// is the lane's, and nothing else is.
    fn mcp_for(&self, name: &str) -> Option<&Arc<McpTools>> {
        self.mcp.as_ref().filter(|_| McpTools::owns(name))
    }

    /// The plan gate, when it owns `name` — the gate's own name table, so a
    /// dispatch and an advertisement cannot disagree about which tool a word
    /// names.
    fn plan_for(&self, name: &str) -> Option<&Arc<PlanGate>> {
        self.plan.as_ref().filter(|_| PlanGate::owns(name))
    }

    /// The `[tools] allow` gate for the members this wrapper serves itself:
    /// the shell and the MCP lane ride no composite, so a name is checked here
    /// before the call is routed — which is what makes `[tools] allow` able to
    /// govern an `mcp__` name. Every other name is checked by the shared
    /// composite below.
    fn admits(&self, name: &str) -> bool {
        self.inner.admits(name)
            && (!shell_policy::reaches_background(name)
                || self
                    .background_started
                    .load(std::sync::atomic::Ordering::Acquire))
    }

    /// Notes a call's outcome: a `shell` call that started a background run is
    /// what puts `shell_output` and `shell_kill` in the list from the next
    /// request on. A failed call advertises nothing.
    fn note_start(&self, started: bool, outcome: &Result<serde_json::Value, ToolError>) {
        if started && outcome.is_ok() {
            self.background_started
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }
}

#[async_trait]
impl ToolExecutor for SessionTools {
    /// The session's registry is shared by every turn's shell, so the turn
    /// that follows an exit sees the notice even though its own shell was
    /// composed after the run started.
    fn take_background_exits(&self) -> Vec<saya_agent::BackgroundExit> {
        self.shell
            .as_ref()
            .map(|shell| shell.take_background_exits())
            .unwrap_or_default()
    }

    fn background_live(&self) -> bool {
        self.shell
            .as_ref()
            .is_some_and(|shell| shell.background_live())
    }

    fn background_notice_pending(&self) -> bool {
        self.shell
            .as_ref()
            .is_some_and(|shell| shell.background_notice_pending())
    }

    async fn wait_background_exit(
        &self,
        deadline: std::time::Instant,
        cancellation: &saya_agent::CancellationToken,
    ) -> bool {
        match &self.shell {
            Some(shell) => shell.wait_background_exit(deadline, cancellation).await,
            None => false,
        }
    }

    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if !self.admits(name) {
            return Err(ToolError::UnsupportedTool);
        }
        if let Some(shell) = self.shell_for(name) {
            let started = shell_policy::starts_background(name, &arguments);
            let outcome = shell.execute(name, arguments).await;
            self.note_start(started, &outcome);
            return outcome;
        }
        if let Some(mcp) = self.mcp_for(name) {
            return mcp.execute(name, arguments).await;
        }
        if let Some(plan) = self.plan_for(name) {
            return plan.execute(name, arguments).await;
        }
        self.inner.execute(name, arguments).await
    }

    async fn execute_with_result_cap(
        &self,
        name: &str,
        arguments: serde_json::Value,
        result_cap: usize,
    ) -> Result<serde_json::Value, ToolError> {
        if !self.admits(name) {
            return Err(ToolError::UnsupportedTool);
        }
        if let Some(shell) = self.shell_for(name) {
            let started = shell_policy::starts_background(name, &arguments);
            let outcome = shell
                .execute_with_result_cap(name, arguments, result_cap)
                .await;
            self.note_start(started, &outcome);
            return outcome;
        }
        if let Some(mcp) = self.mcp_for(name) {
            return mcp
                .execute_with_result_cap(name, arguments, result_cap)
                .await;
        }
        if let Some(plan) = self.plan_for(name) {
            return plan
                .execute_with_result_cap(name, arguments, result_cap)
                .await;
        }
        self.inner
            .execute_with_result_cap(name, arguments, result_cap)
            .await
    }

    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        if !self.admits(name) {
            return Err(ToolError::UnsupportedTool);
        }
        if let Some(shell) = self.shell_for(name) {
            let started = shell_policy::starts_background(name, &arguments);
            let outcome = shell
                .execute_with_context(name, arguments, context)
                .await;
            self.note_start(started, &outcome);
            return outcome;
        }
        if let Some(mcp) = self.mcp_for(name) {
            return mcp.execute_with_context(name, arguments, context).await;
        }
        if let Some(plan) = self.plan_for(name) {
            return plan.execute_with_context(name, arguments, context).await;
        }
        self.inner
            .execute_with_context(name, arguments, context)
            .await
    }
}
