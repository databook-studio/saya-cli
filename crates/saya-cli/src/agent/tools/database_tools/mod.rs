use std::sync::Arc;
use std::time::Duration;

use saya_harness::workspace::Workspace;
use saya_store::SqliteStateStore;

use crate::connection::ConnectionRegistry;
use crate::contracts::RecallReceipt;

mod chart_tool;
mod column_health;
mod definitions;
mod dispatch;
mod fan_out;
mod join_check;
mod observations;
mod recorder;
mod result_shape;
// A1: request-scoped log of override findings. Mirrors `propose/log.rs`; the
// runtime drains it after the loop to emit one `KnowledgeOverridden` event.
mod override_log;
mod redaction_guard;
mod workspace_edit;
mod workspace_edit_anchor;
mod workspace_edit_append;
mod workspace_edit_args;
mod workspace_edit_replace;
mod workspace_edit_shared;
mod workspace_glob;
mod workspace_grep;
mod workspace_list;
mod workspace_read;
mod workspace_write;

// `ObservationLog` types the `observations` field; the observation records and
// the drained log are re-exported so the agent runtime's learning wiring
// (`agent::learning`) and the sibling integration test (`observations_tests`)
// can reach them without the private `observations` submodule being public.
pub(crate) use observations::{
    DrainedObservations, ObservationLog, ObservationOutcome, ToolObservation,
};
// `OverrideLog` types the `override_log` field; re-exported so the runtime can
// drain it to emit one `KnowledgeOverridden` event.
pub(crate) use override_log::OverrideLog;
// The workspace tool bounds type the workspace tools' harness arguments.
// Re-exported for tests only, so the sibling tests build oversized files and
// over-bound directories against the exact bounds rather than copies of them
// that can go stale. Gated rather than `allow(unused_imports)`: the imports
// genuinely are test-only, and saying so is better than silencing the lint
// that noticed.
#[cfg(test)]
pub(crate) use workspace_glob::{WORKSPACE_GLOB_MAX_MATCHES, WORKSPACE_GLOB_MAX_VISITED};
#[cfg(test)]
pub(crate) use workspace_grep::{
    WORKSPACE_GREP_MAX_FILE_BYTES, WORKSPACE_GREP_MAX_LINE_BYTES, WORKSPACE_GREP_MAX_MATCHES,
    WORKSPACE_GREP_MAX_VISITED,
};
#[cfg(test)]
pub(crate) use workspace_list::WORKSPACE_LIST_MAX_ENTRIES;
#[cfg(test)]
pub(crate) use workspace_read::WORKSPACE_READ_MAX_BYTES;
// `WORKSPACE_WRITE_MAX_BYTES` is not test-gated: the approval prompt states
// the exact per-write bound the tool enforces (`approval_facts`), so the
// prompt's number is this constant, never a copy of it.
#[cfg(test)]
pub(crate) use workspace_edit::WORKSPACE_EDIT_MAX_BYTES;
pub(crate) use workspace_write::WORKSPACE_WRITE_MAX_BYTES;

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
    /// no detection — never a guess. An `Arc` so the runtime and the
    /// tools share one reference.
    pub(super) recall_receipt: Option<Arc<RecallReceipt>>,
    /// Request-scoped log of override findings, drained by the runtime to emit
    /// one `KnowledgeOverridden` event. `None` in tests; an absent log
    /// means no event, never a side effect. An `Arc` so the runtime can drain
    /// after the tools consume their clone.
    pub(super) override_log: Option<Arc<OverrideLog>>,
    /// The run's contained workspace, the only file I/O a model-facing tool
    /// reaches. Path resolution is delegated entirely to `Workspace::read` —
    /// nothing here resolves a path itself. `None` (every current path, until
    /// the run engine passes one in) leaves `workspace_read` denying with a
    /// typed error: no workspace, no read. An `Arc` so the engine's handle and
    /// the tools share one resolved root.
    pub(super) workspace: Option<Arc<Workspace>>,
}

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
            supplied_objects: Vec::new(),
            recall_receipt: None,
            override_log: None,
            workspace: None,
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
            workspace: None,
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
            workspace: None,
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

    /// Attaches the run's contained workspace. `None` (every current caller,
    /// until the run engine resolves and opens one) leaves `workspace_read`
    /// denying with a typed error — the definition is advertised, the dispatch
    /// refuses.
    pub(crate) fn with_workspace(mut self, workspace: Option<Arc<Workspace>>) -> Self {
        self.workspace = workspace;
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
            workspace: None,
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
            workspace: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_agent::ToolExecutor;

    #[test]
    fn render_chart_requires_approval() {
        let tools = DatabaseTools::definitions(true, false, false, false, true);
        let chart_tool = tools
            .iter()
            .find(|tool| tool.name == "render_chart")
            .expect("render_chart definition exists");
        assert!(chart_tool.effect.requires_approval);
        assert!(chart_tool.effect.external_side_effect);
        assert!(!chart_tool.effect.database_data);
    }

    #[test]
    fn render_chart_offers_workspace_save_only_with_write_permit() {
        let tools =
            DatabaseTools::definitions_with_chart_save(true, false, false, true, true, true);
        let chart_tool = tools
            .iter()
            .find(|tool| tool.name == "render_chart")
            .unwrap();
        assert_eq!(
            chart_tool.parameters["properties"]["save_to"]["type"],
            "string"
        );
        assert_eq!(
            chart_tool.effect.local_state,
            saya_agent::LocalStateEffect::WriteWorkspace
        );
        assert!(chart_tool.effect.requires_approval);
        assert!(chart_tool.effect.external_side_effect);

        let tools =
            DatabaseTools::definitions_with_chart_save(true, false, false, false, true, false);
        let chart_tool = tools
            .iter()
            .find(|tool| tool.name == "render_chart")
            .unwrap();
        assert!(chart_tool.parameters["properties"].get("save_to").is_none());
        assert_eq!(
            chart_tool.effect.local_state,
            saya_agent::LocalStateEffect::None
        );
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
        assert_eq!(path.parent(), Some(std::env::temp_dir().as_path()));
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("saya-chart-")
        );
        let meta = std::fs::metadata(path).expect("file should exist");

        use std::os::unix::fs::PermissionsExt;
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_file(path);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn render_chart_saves_contained_html_without_returning_rows() {
        use async_trait::async_trait;
        use saya_connectors::DatabaseConnector;
        use saya_harness::workspace::Workspace;
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

        let root = std::env::temp_dir().join(format!(
            "saya-chart-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock is after epoch")
                .as_nanos()
        ));
        std::fs::create_dir(&root).expect("workspace tempdir");
        let mut tools = DatabaseTools::new(Some(Box::new(NonEmptyConnector)), 100, true);
        tools.workspace = Some(Arc::new(Workspace::open(&root).expect("workspace opens")));
        let res = tools
            .execute(
                "render_chart",
                serde_json::json!({
                    "sql": "SELECT 1",
                    "chart_type": "bar",
                    "save_to": "monthly.html"
                }),
            )
            .await
            .expect("render_chart should save");

        assert_eq!(res["path"], "monthly.html");
        assert_eq!(res.as_object().unwrap().len(), 2);
        let saved = root.join("monthly.html");
        let bytes = std::fs::read(&saved).expect("contained chart exists");
        let result = QueryResult {
            columns: vec!["cat".into(), "val".into()],
            rows: vec![serde_json::json!(["A", 10])],
            row_count: 1,
            truncated: false,
            executed_sql: "SELECT 1".into(),
        };
        let mut spec = crate::chart::suggest_spec(&result);
        spec.kind = crate::chart::ChartKind::Bar;
        let expected = crate::chart::render_html(&result, &spec).expect("chart renders");
        assert_eq!(bytes, expected.as_bytes());

        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(saved).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let outside = root
            .parent()
            .unwrap()
            .join(format!("saya-chart-outside-{}", std::process::id()));
        std::fs::write(&outside, b"sentinel").expect("outside sentinel");
        let paths = [
            format!("../{}", outside.file_name().unwrap().to_string_lossy()),
            outside.display().to_string(),
        ];
        for path in &paths {
            let error = tools
                .execute(
                    "render_chart",
                    serde_json::json!({
                        "sql": "SELECT 1",
                        "chart_type": "bar",
                        "save_to": path
                    }),
                )
                .await
                .expect_err("escaping chart path must refuse");
            assert!(matches!(error, saya_agent::ToolError::WorkspaceWrite(_)));
        }
        assert_eq!(std::fs::read(&outside).unwrap(), b"sentinel");
        let symlink = root.join("linked.html");
        std::os::unix::fs::symlink(&outside, &symlink).expect("workspace symlink");
        let error = tools
            .execute(
                "render_chart",
                serde_json::json!({
                    "sql": "SELECT 1",
                    "chart_type": "bar",
                    "save_to": "linked.html"
                }),
            )
            .await
            .expect_err("symlink chart path must refuse");
        assert!(matches!(error, saya_agent::ToolError::WorkspaceWrite(_)));
        assert_eq!(std::fs::read(&outside).unwrap(), b"sentinel");
        std::fs::remove_file(outside).expect("outside sentinel cleans up");
        tools.workspace = None;
        let error = tools
            .execute(
                "render_chart",
                serde_json::json!({
                    "sql": "SELECT 1",
                    "chart_type": "bar",
                    "save_to": "missing-workspace.html"
                }),
            )
            .await
            .expect_err("saving without a bound workspace must refuse");
        assert_eq!(error, saya_agent::ToolError::WorkspaceUnavailable);
        std::fs::remove_dir_all(root).expect("workspace cleans up");
    }

    #[tokio::test]
    async fn render_chart_over_bound_does_not_replace_saved_file() {
        use async_trait::async_trait;
        use saya_connectors::DatabaseConnector;
        use saya_harness::workspace::{MAX_IO_BYTES, Workspace};
        use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

        struct LargeConnector;
        #[async_trait]
        impl DatabaseConnector for LargeConnector {
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
                    columns: vec!["large".into()],
                    rows: vec![serde_json::json!(["x".repeat(MAX_IO_BYTES + 1)])],
                    row_count: 1,
                    truncated: false,
                    executed_sql: req.sql,
                })
            }
        }

        let root = std::env::temp_dir().join(format!("saya-chart-bound-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).expect("workspace tempdir");
        let destination = root.join("chart.html");
        std::fs::write(&destination, b"old chart").expect("existing destination");
        let mut tools = DatabaseTools::new(Some(Box::new(LargeConnector)), 100, true);
        tools.workspace = Some(Arc::new(Workspace::open(&root).expect("workspace opens")));

        let error = tools
            .execute(
                "render_chart",
                serde_json::json!({
                    "sql": "SELECT large",
                    "chart_type": "bar",
                    "save_to": "chart.html"
                }),
            )
            .await
            .expect_err("over-bound HTML must refuse");
        assert!(matches!(error, saya_agent::ToolError::WorkspaceWrite(_)));
        assert_eq!(std::fs::read(destination).unwrap(), b"old chart");
        std::fs::remove_dir_all(root).expect("workspace cleans up");
    }
}
