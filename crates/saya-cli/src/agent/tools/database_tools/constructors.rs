impl DatabaseTools {
    const MAX_CONCURRENT_FAN_OUT_QUERIES: usize = 4;
    /// The wall-clock ceiling one fan-out query runs under (`fan_out.rs`
    /// wraps every per-database query in it, narrowing the connector's own
    /// configured timeout). `pub(crate)` so the approval prompt states the
    /// same figure the fan-out applies — the prompt's number and the
    /// enforcement are the same constant by construction.
    pub(crate) const FAN_OUT_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

    /// Creates database tools with a single optional primary connection for testing.
    #[cfg(test)]
    pub(crate) fn new(
        connector: Option<Box<dyn saya_connectors::DatabaseConnector>>,
        max_rows: usize,
        allow_query_data: bool,
    ) -> Self {
        use crate::connection::ConnectionEntry;

        let mut registry = ConnectionRegistry::new("primary");
        if let Some(c) = connector {
            let dialect = c.dialect();
            registry.insert(
                "primary",
                ConnectionEntry {
                    connector: c,
                    dialect,
                    profile_id: None,
                },
            );
        }
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db: None,
            max_concurrent_fan_out_queries: Self::MAX_CONCURRENT_FAN_OUT_QUERIES,
            fan_out_query_timeout: Self::FAN_OUT_QUERY_TIMEOUT,
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

    /// Creates database tools configured with a connection registry, with no
    /// observation log attached. Used by tests that drive tools without a
    /// learning setup; the production path uses [`Self::with_learning`].
    #[cfg(test)]
    pub(crate) fn with_registry(
        registry: ConnectionRegistry,
        max_rows: usize,
        allow_query_data: bool,
        state_db: Option<SqliteStateStore>,
    ) -> Self {
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db,
            max_concurrent_fan_out_queries: Self::MAX_CONCURRENT_FAN_OUT_QUERIES,
            fan_out_query_timeout: Self::FAN_OUT_QUERY_TIMEOUT,
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

    /// Production construction with a learning-derived observation log attached.
    /// `observations` is `None` for `learning = off` (no collector exists); `Some`
    /// for `assisted`, so the runtime can drain it after the turn for extraction.
    pub(crate) fn with_learning(
        registry: ConnectionRegistry,
        max_rows: usize,
        allow_query_data: bool,
        state_db: Option<SqliteStateStore>,
        observations: Option<Arc<ObservationLog>>,
    ) -> Self {
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db,
            max_concurrent_fan_out_queries: Self::MAX_CONCURRENT_FAN_OUT_QUERIES,
            fan_out_query_timeout: Self::FAN_OUT_QUERY_TIMEOUT,
            observations,
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
