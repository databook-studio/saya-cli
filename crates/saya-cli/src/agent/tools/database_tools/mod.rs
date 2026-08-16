use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use saya_store::SqliteStateStore;

use crate::connection::ConnectionRegistry;

mod chart_tool;
mod definitions;
mod dispatch;
mod fan_out;
mod observations;
// `pub(super)` so the sibling `definitions` module can reach the tool definition.
pub(super) mod propose;
mod recorder;

// `ObservationLog` types the `observations` field; the observation records and
// the drained log are re-exported so the agent runtime's learning wiring
// (`agent::learning`) and the sibling integration test (`observations_tests`)
// can reach them without the private `observations` submodule being public.
pub(crate) use observations::{
    DrainedObservations, ObservationLog, ObservationOutcome, ToolObservation,
};
// `ProposedClaimsLog` types the `proposed_claims` field; re-exported the same
// way so the agent runtime can drain it after the turn to emit one
// `KnowledgeProposed` event per persisted claim (spec P2d).
pub(crate) use propose::ProposedClaimsLog;

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
    /// Per-request count of candidate proposals made this turn. `contract_propose`
    // refuses the ninth (spec 3c §2). A `DatabaseTools` is constructed once per
    // `run_prompt_with_sink` call and shared by `&self` across the loop, so this
    // is the request scope — not global — the bound is meant to cover. Atomic so
    // the `&self` executor can bump it without `&mut self`.
    pub(super) candidate_proposals: AtomicUsize,
    /// Request-scoped log of the candidate claims *persisted* this turn, for
    // the runtime to drain and emit as one `KnowledgeProposed` event per claim
    // (spec P2d). `None` in tests that drive the executor without a log; an
    // absent log means no event is emitted, it never affects persistence. An
    // `Arc` lets the runtime keep a handle to drain after the turn.
    pub(super) proposed_claims: Option<Arc<ProposedClaimsLog>>,
    /// Qualified `catalog.schema.object` names of objects whose claims were
    /// supplied to the model this turn via recall.
    ///
    /// A proposal for any object in this list must not earn the strong `TOUCHED`
    /// evidence kind even if a query touched it this turn: the query was caused
    /// by the supplied claim and is not independent confirmation.
    pub(super) supplied_objects: Vec<String>,
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
            candidate_proposals: AtomicUsize::new(0),
            proposed_claims: None,
            supplied_objects: Vec::new(),
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
            candidate_proposals: AtomicUsize::new(0),
            proposed_claims: None,
            supplied_objects: Vec::new(),
        }
    }

    /// Production construction with a learning-derived observation log attached.
    /// `observations` is `None` for `learning = off` (no collector exists); `Some`
    /// for `suggest` and `auto-candidate`, so the runtime can drain it after the
    /// turn to report or persist what was observed (spec 4b §2). `proposed_claims`
    /// is the request-scoped log `contract_propose` records a persisted claim
    /// into, which the runtime drains after the turn to emit one
    /// `KnowledgeProposed` event per claim (spec P2d). `None` only when no store
    /// is present (no proposals can persist); the runtime drains it regardless
    /// of whether the turn succeeded — a persisted write is reported even when
    /// the turn later fails (spec P2d §3).
    pub(crate) fn with_learning(
        registry: ConnectionRegistry,
        max_rows: usize,
        allow_query_data: bool,
        state_db: Option<SqliteStateStore>,
        observations: Option<Arc<ObservationLog>>,
        proposed_claims: Option<Arc<ProposedClaimsLog>>,
    ) -> Self {
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db,
            max_concurrent_fan_out_queries: Self::MAX_CONCURRENT_FAN_OUT_QUERIES,
            fan_out_query_timeout: Self::FAN_OUT_QUERY_TIMEOUT,
            observations,
            candidate_proposals: AtomicUsize::new(0),
            proposed_claims,
            supplied_objects: Vec::new(),
        }
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
            candidate_proposals: AtomicUsize::new(0),
            proposed_claims: None,
            supplied_objects: Vec::new(),
        }
    }

    /// Test-only construction with a shared observation log attached, so a test
    // can drive tools through `execute` and then `drain` the same log. An
    // optional `proposed_claims` log lets a test assert what a persisted proposal
    // records for a `KnowledgeProposed` event (spec P2d §5); absent, no event data
    // is captured.
    #[cfg(test)]
    pub(super) fn with_registry_and_observations(
        registry: ConnectionRegistry,
        max_rows: usize,
        allow_query_data: bool,
        state_db: Option<SqliteStateStore>,
        observations: Arc<ObservationLog>,
        proposed_claims: Option<Arc<ProposedClaimsLog>>,
    ) -> Self {
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db,
            max_concurrent_fan_out_queries: Self::MAX_CONCURRENT_FAN_OUT_QUERIES,
            fan_out_query_timeout: Self::FAN_OUT_QUERY_TIMEOUT,
            observations: Some(observations),
            candidate_proposals: AtomicUsize::new(0),
            proposed_claims,
            supplied_objects: Vec::new(),
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
