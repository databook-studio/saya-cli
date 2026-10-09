impl DatabaseTools {
    /// Attaches the turn's question-scope metrics collector. `None` — the
    /// default in every constructor — records nothing; behavior is otherwise
    /// identical with and without a collector.
    pub(crate) fn with_metrics(mut self, metrics: Option<Arc<LocalMetrics>>) -> Self {
        self.metrics = metrics;
        self
    }

    /// Returns a reference to the active connection registry.
    pub(crate) fn registry(&self) -> &ConnectionRegistry {
        &self.registry
    }

    /// Returns a reference to the optional state database.
    pub(crate) fn state_db(&self) -> Option<&SqliteStateStore> {
        self.state_db.as_ref()
    }

    /// Attaches the turn's supplied qualified object names (from `RecallReceipt::supplied`).
    pub(crate) fn with_supplied_objects(mut self, supplied_objects: Vec<String>) -> Self {
        self.supplied_objects = supplied_objects;
        self
    }

    /// Attaches the run's contained workspace. `None` (every current caller,
    /// until the run engine resolves and opens one) leaves `workspace_read`
    /// denying with a typed error — the definition is advertised, the dispatch
    /// refuses.
    pub(crate) fn with_workspace(mut self, workspace: Option<Arc<Workspace>>) -> Self {
        self.workspace = workspace;
        self
    }

    /// Attaches the TUI's capture hook (C1) together with the turn's context
    /// byte budget for direct executor calls. The agent loop overrides it with
    /// its exact per-call result share. Only the `bounded_sql_query` arm
    /// consults it, only on success; every headless path passes `None` and
    /// nothing captures.
    pub(crate) fn with_capture(
        mut self,
        capture_hook: Option<CaptureHook>,
        context_byte_budget: usize,
    ) -> Self {
        self.capture_hook = capture_hook;
        self.context_byte_budget = context_byte_budget;
        self
    }

    /// Attaches an already-built checkpoint recorder for direct file-tool
    /// edits. `None` — every construction that has not attached one —
    /// leaves behaviour identical: no capture, no cleanup, no `recovery`
    /// field in any result. The session wiring attaches through
    /// [`Self::with_recovery`]; the sibling tests build recorders here
    /// directly.
    pub(in crate::agent::tools) fn with_checkpoints(
        mut self,
        checkpoints: Option<super::workspace::checkpoint::CheckpointRecorder>,
    ) -> Self {
        self.checkpoints = checkpoints;
        self
    }

    /// Attaches the session's checkpoint recorder from its recovery
    /// parts — the store, the session id, and the session-scoped enable
    /// flag. `None` — the one-shot `ask` path, `saya run`, and every
    /// non-session construction — leaves the field `None`: no capture,
    /// no `recovery` field in any result.
    pub(crate) fn with_recovery(
        self,
        parts: Option<(Arc<FsSessionStore>, String, Arc<AtomicBool>)>,
    ) -> Self {
        self.with_checkpoints(parts.map(|(store, session_id, enabled)| {
            super::workspace::checkpoint::CheckpointRecorder::new(store, session_id, enabled)
        }))
    }

    #[cfg(test)]
    pub(super) fn with_registry_and_fan_out_limits(
        registry: ConnectionRegistry,
        max_rows: usize,
        allow_query_data: bool,
        max_concurrent_fan_out_queries: usize,
        fan_out_query_timeout: Duration,
    ) -> Self {
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db: None,
            max_concurrent_fan_out_queries: max_concurrent_fan_out_queries.max(1),
            fan_out_query_timeout,
            observations: None,
            metrics: None,
            supplied_objects: Vec::new(),
            recall_receipt: None,
            override_log: None,
            workspace: None,
            capture_hook: None,
            checkpoints: None,
            context_byte_budget: saya_agent::AgentLimits::default().context_byte_budget,
        }
    }

    /// Test-only construction with a shared observation log attached.
    #[cfg(test)]
    pub(super) fn with_registry_and_observations(
        registry: ConnectionRegistry,
        max_rows: usize,
        allow_query_data: bool,
        state_db: Option<SqliteStateStore>,
        observations: Arc<ObservationLog>,
    ) -> Self {
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db,
            max_concurrent_fan_out_queries: Self::MAX_CONCURRENT_FAN_OUT_QUERIES,
            fan_out_query_timeout: Self::FAN_OUT_QUERY_TIMEOUT,
            observations: Some(observations),
            metrics: None,
            supplied_objects: Vec::new(),
            recall_receipt: None,
            override_log: None,
            workspace: None,
            capture_hook: None,
            checkpoints: None,
            context_byte_budget: saya_agent::AgentLimits::default().context_byte_budget,
        }
    }
}
