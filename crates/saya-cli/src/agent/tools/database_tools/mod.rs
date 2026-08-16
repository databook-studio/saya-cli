use std::sync::Arc;
use std::time::Duration;

use saya_store::SqliteStateStore;

use crate::connection::ConnectionRegistry;
use crate::contracts::RecallReceipt;

mod chart_tool;
mod definitions;
mod dispatch;
mod fan_out;
mod observations;
mod recorder;
// A1: request-scoped log of override findings. Mirrors `propose/log.rs`; the
// runtime drains it after the loop to emit one `KnowledgeOverridden` event.
mod override_log;

// `ObservationLog` types the `observations` field; the observation records and
// the drained log are re-exported so the agent runtime's learning wiring
// (`agent::learning`) and the sibling integration test (`observations_tests`)
// can reach them without the private `observations` submodule being public.
pub(crate) use observations::{
    DrainedObservations, ObservationLog, ObservationOutcome, ToolObservation,
};
// `OverrideLog` types the `override_log` field; re-exported so the runtime can
// drain it to emit one `KnowledgeOverridden` event (spec A1).
pub(crate) use override_log::OverrideLog;

/// Agent tools for inspecting and querying configured database connections.
pub(crate) struct DatabaseTools {
    // `pub(super)` so the sibling `contract_tools` module (also a child of
    // `agent::tools`) can resolve a connection and read the privacy/store flags
    // without re-deriving provider policy.
    pub(super) registry: ConnectionRegistry,
    pub(super) max_rows: usize,
    pub(super) allow_query_data: bool,
    pub(super) state_db: Option<SqliteStateStore>,
    pub(super) max_concurrent_fan_out_queries: usize,
    pub(super) fan_out_query_timeout: Duration,
    /// Request-scoped observation collector. `None` leaves behaviour identical
    // to before Phase 3b-2; recording is a no-op when absent (spec §4). An
    // `Arc` lets the application operation that creates the log keep a handle to
    // drain it after the turn while the tools hold their own reference.
    pub(super) observations: Option<Arc<ObservationLog>>,
    /// Qualified `catalog.schema.object` names of objects whose claims were
    /// supplied to the model this turn via recall.
    pub(super) supplied_objects: Vec<String>,
    /// The turn's recall receipt, shared with the override detector. `None` in
    /// tests that drive the executor without a receipt; an absent receipt means
    /// no detection — never a guess (spec A1 §3). An `Arc` so the runtime and the
    /// tools share one reference.
    pub(super) recall_receipt: Option<Arc<RecallReceipt>>,
    /// Request-scoped log of override findings, drained by the runtime to emit
    /// one `KnowledgeOverridden` event (spec A1). `None` in tests; an absent log
    /// means no event, never a side effect. An `Arc` so the runtime can drain
    /// after the tools consume their clone.
    pub(super) override_log: Option<Arc<OverrideLog>>,
}

impl DatabaseTools {
    const MAX_CONCURRENT_FAN_OUT_QUERIES: usize = 4;
    const FAN_OUT_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

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
            supplied_objects: Vec::new(),
            recall_receipt: None,
            override_log: None,
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
            supplied_objects: Vec::new(),
            recall_receipt: None,
            override_log: None,
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
            supplied_objects: Vec::new(),
            recall_receipt: None,
            override_log: None,
        }
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
            supplied_objects: Vec::new(),
            recall_receipt: None,
            override_log: None,
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
            supplied_objects: Vec::new(),
            recall_receipt: None,
            override_log: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_agent::ToolExecutor;

    #[test]
    fn render_chart_requires_approval() {
        let tools = DatabaseTools::definitions(true, false, false);
        let chart_tool = tools
            .iter()
            .find(|tool| tool.name == "render_chart")
            .expect("render_chart definition exists");
        assert!(chart_tool.effect.requires_approval);
        assert!(chart_tool.effect.external_side_effect);
        assert!(!chart_tool.effect.database_data);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn render_chart_creates_0600_permissions_file() {
        use async_trait::async_trait;
        use saya_connectors::DatabaseConnector;
        use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

        struct NonEmptyConnector;

        #[async_trait]
        impl DatabaseConnector for NonEmptyConnector {
            fn dialect(&self) -> SqlDialect {
                SqlDialect::DuckDb
            }

            async fn connect(&self) -> Result<(), ConnectionError> {
                Ok(())
            }

            async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
                Ok(SchemaTree::default())
            }

            async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
                Ok(QueryResult {
                    columns: vec!["cat".into(), "val".into()],
                    rows: vec![serde_json::json!(["A", 10])],
                    row_count: 1,
                    truncated: false,
                    executed_sql: req.sql,
                })
            }
        }

        let tools = DatabaseTools::new(Some(Box::new(NonEmptyConnector)), 100, true);
        let res = tools
            .execute(
                "render_chart",
                serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"}),
            )
            .await
            .expect("render_chart should succeed");

        let path_str = res["path"].as_str().expect("path in response");
        let path = std::path::Path::new(path_str);
        let meta = std::fs::metadata(path).expect("file should exist");

        use std::os::unix::fs::PermissionsExt;
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_file(path);
    }
}
