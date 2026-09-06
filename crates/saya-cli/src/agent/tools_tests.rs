use super::*;
use async_trait::async_trait;
use saya_agent::ToolExecutor;
use saya_connectors::DatabaseConnector;
use saya_types::{
    ConnectionError, Database, QueryRequest, QueryResult, Schema, SchemaTree, SqlDialect, Table,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Barrier;

struct FakeConnector {
    table_name: String,
}

#[async_trait]
impl DatabaseConnector for FakeConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree {
            databases: vec![Database {
                name: "main".into(),
                schemas: vec![Schema {
                    name: "public".into(),
                    tables: vec![Table {
                        name: self.table_name.clone(),
                        primary_key: vec![],
                        foreign_keys: vec![],
                        columns: vec![],
                    }],
                }],
            }],
        })
    }

    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(req.sql))
    }
}

/// A connector whose queries always fail, used to exercise the fan-out
/// per-database error path (e.g. a dialect mismatch on one database).
struct FailingConnector {
    dialect: SqlDialect,
}

struct BarrierConnector {
    barrier: Arc<Barrier>,
}

struct SlowConnector;

#[async_trait]
impl DatabaseConnector for SlowConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }

    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(QueryResult::empty("SELECT 1"))
    }
}

#[async_trait]
impl DatabaseConnector for BarrierConnector {
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
        self.barrier.wait().await;
        Ok(QueryResult::empty(req.sql))
    }
}

#[async_trait]
impl DatabaseConnector for FailingConnector {
    fn dialect(&self) -> SqlDialect {
        self.dialect
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Err(ConnectionError::schema_failed("nope"))
    }

    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Err(ConnectionError::query_failed("syntax error near FROM"))
    }
}

fn two_connection_registry() -> ConnectionRegistry {
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(FakeConnector {
                table_name: "from_primary".into(),
            }),
            dialect: SqlDialect::DuckDb,
            profile_id: None,
        },
    );
    registry.insert(
        "warehouse",
        ConnectionEntry {
            connector: Box::new(FakeConnector {
                table_name: "from_secondary".into(),
            }),
            dialect: SqlDialect::Postgres,
            profile_id: None,
        },
    );
    registry
}

#[tokio::test]
async fn bounded_sql_query_all_fans_out_over_every_connection() {
    let tools = DatabaseTools::with_registry(two_connection_registry(), 100, true, None);

    let res = tools
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1"}),
        )
        .await
        .expect("fan-out query should succeed");

    let databases = res
        .get("databases")
        .and_then(|value| value.as_array())
        .expect("result should carry a `databases` array");
    assert_eq!(databases.len(), 2, "one entry per connected database");

    let names: Vec<&str> = databases
        .iter()
        .filter_map(|db| db.get("connection").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        names,
        vec!["primary", "warehouse"],
        "insertion order preserved"
    );
    // Every database reports its dialect and a successful result, not an error.
    for db in databases {
        assert!(db.get("dialect").is_some(), "each entry names its dialect");
        assert!(db.get("result").is_some(), "each entry carries a result");
        assert!(db.get("error").is_none(), "no errors expected here");
    }
}

#[tokio::test]
async fn bounded_sql_query_all_reports_per_database_errors_without_aborting() {
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(FakeConnector {
                table_name: "ok".into(),
            }),
            dialect: SqlDialect::DuckDb,
            profile_id: None,
        },
    );
    registry.insert(
        "snowflake",
        ConnectionEntry {
            connector: Box::new(FailingConnector {
                dialect: SqlDialect::Snowflake,
            }),
            dialect: SqlDialect::Snowflake,
            profile_id: None,
        },
    );
    let tools = DatabaseTools::with_registry(registry, 100, true, None);

    let res = tools
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1"}),
        )
        .await
        .expect("fan-out should succeed even when one database fails");

    let databases = res["databases"].as_array().expect("databases array");
    assert_eq!(databases.len(), 2);
    assert!(
        databases[0].get("result").is_some(),
        "the healthy database still returns a result"
    );
    assert!(
        databases[1].get("error").is_some(),
        "the failing database reports an error instead of sinking the run"
    );
    assert!(
        databases[1]["error"]
            .as_str()
            .is_some_and(|error| error.contains("syntax error near FROM")),
        "safe connector detail should identify the dialect mismatch"
    );
}

#[tokio::test]
async fn bounded_sql_query_all_runs_connections_concurrently() {
    let barrier = Arc::new(Barrier::new(2));
    let mut registry = ConnectionRegistry::new("primary");
    for name in ["primary", "warehouse"] {
        registry.insert(
            name,
            ConnectionEntry {
                connector: Box::new(BarrierConnector {
                    barrier: barrier.clone(),
                }),
                dialect: SqlDialect::DuckDb,
                profile_id: None,
            },
        );
    }
    let tools = DatabaseTools::with_registry(registry, 100, true, None);

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        tools.execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1"}),
        ),
    )
    .await;
    assert!(
        result.is_ok(),
        "both queries must start before either can finish"
    );
}

#[tokio::test]
async fn bounded_sql_query_all_respects_its_concurrency_cap() {
    let barrier = Arc::new(Barrier::new(2));
    let mut registry = ConnectionRegistry::new("primary");
    for name in ["primary", "warehouse"] {
        registry.insert(
            name,
            ConnectionEntry {
                connector: Box::new(BarrierConnector {
                    barrier: barrier.clone(),
                }),
                dialect: SqlDialect::DuckDb,
                profile_id: None,
            },
        );
    }
    let tools = DatabaseTools::with_registry_and_fan_out_limits(
        registry,
        100,
        true,
        1,
        Duration::from_secs(1),
    );

    assert!(
        tokio::time::timeout(
            Duration::from_millis(50),
            tools.execute(
                "bounded_sql_query_all",
                serde_json::json!({"sql": "SELECT 1"}),
            ),
        )
        .await
        .is_err(),
        "a cap of one must not start the second barrier participant"
    );
}

#[tokio::test]
async fn bounded_sql_query_all_reports_per_database_timeouts() {
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(SlowConnector),
            dialect: SqlDialect::DuckDb,
            profile_id: None,
        },
    );
    let tools = DatabaseTools::with_registry_and_fan_out_limits(
        registry,
        100,
        true,
        1,
        Duration::from_millis(10),
    );

    let result = tools
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1"}),
        )
        .await
        .expect("fan-out timeout is reported per database");
    assert_eq!(result["databases"][0]["error"], "read-only query timed out");
}

#[tokio::test]
async fn bounded_sql_query_all_is_blocked_when_data_sharing_is_disabled() {
    let tools = DatabaseTools::with_registry(two_connection_registry(), 100, false, None);
    let err = tools
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1"}),
        )
        .await
        .expect_err("fan-out must respect the data-sharing guard");
    assert!(
        err.to_string().contains("data sharing is disabled"),
        "got: {err}"
    );
}

#[tokio::test]
async fn tool_execution_rejects_arguments_outside_its_schema() {
    let tools = DatabaseTools::new(None, 100, true);
    for (name, arguments) in [
        ("schema_discovery", serde_json::json!({"sql": "SELECT 1"})),
        ("bounded_sql_query", serde_json::json!({"sql": 1})),
        (
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT 1", "connection": "primary"}),
        ),
    ] {
        let error = tools
            .execute(name, arguments)
            .await
            .expect_err("invalid tool arguments must not reach a connector");
        assert!(
            error.to_string().contains("invalid tool arguments"),
            "got: {error}"
        );
    }
}

#[test]
fn definitions_include_fan_out_only_when_query_data_allowed() {
    let with_data: Vec<String> = DatabaseTools::definitions(true, false, false)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(with_data.iter().any(|name| name == "bounded_sql_query_all"));

    let without_data: Vec<String> = DatabaseTools::definitions(false, false, false)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(
        !without_data
            .iter()
            .any(|name| name == "bounded_sql_query_all")
    );
    assert!(!without_data.iter().any(|name| name == "bounded_sql_query"));
}

#[test]
fn definitions_preserve_the_read_only_and_approval_contract() {
    let tools = DatabaseTools::definitions(true, false, false);
    let tool = |name: &str| tools.iter().find(|tool| tool.name == name).unwrap();

    let schema = tool("schema_discovery");
    assert!(schema.read_only);
    assert!(!schema.effect.requires_approval);
    assert!(schema.parameters["properties"]["connection"].is_object());
    assert!(schema.parameters.get("required").is_none());

    let single = tool("bounded_sql_query");
    assert!(single.read_only);
    assert!(single.effect.requires_approval);
    assert_eq!(single.parameters["required"], serde_json::json!(["sql"]));
    assert!(single.parameters["properties"]["connection"].is_object());
    assert!(single.parameters["properties"]["sql"].is_object());

    let all = tool("bounded_sql_query_all");
    assert!(all.read_only);
    assert!(all.effect.requires_approval);
    assert_eq!(all.parameters["required"], serde_json::json!(["sql"]));
    assert!(all.parameters["properties"]["sql"].is_object());
    assert!(all.parameters["properties"].get("connection").is_none());
}

/// `designate_answer` nominates the statement that answered the question, not
/// an exploratory probe. Two questions nominated nothing at all and one
/// nominated a probe in the benchmark; the description must say plainly that a
/// probe is never the answering query.
#[test]
fn designate_answer_description_forbids_an_exploratory_probe() {
    let tools = DatabaseTools::definitions(true, false, false);
    let designate = tools
        .iter()
        .find(|tool| tool.name == "designate_answer")
        .expect("designate_answer must be registered when query data is allowed");
    let description = &designate.description;
    assert!(
        description.contains("never an exploratory probe"),
        "the description must plainly forbid nominating a probe: {description}"
    );
    assert!(
        description.contains("the statement that produced the answer"),
        "the description must name the answering statement: {description}"
    );
}

/// Spec 3a §2 / 3c: every existing tool declares the expected `local_state`.
/// This is the test that fails when someone adds a tool without saying what
/// local state it touches. With query data and a state store but candidate
/// writes off, the six pre-3c tools are present and none writes local state.
#[test]
fn every_tool_declares_its_local_state_effect() {
    use saya_agent::LocalStateEffect;

    let tools = DatabaseTools::definitions(true, true, false);
    let expected = [
        ("schema_discovery", LocalStateEffect::None),
        ("bounded_sql_query", LocalStateEffect::None),
        ("bounded_sql_query_all", LocalStateEffect::None),
        ("result_shape", LocalStateEffect::None),
        ("column_health", LocalStateEffect::None),
        ("join_check", LocalStateEffect::None),
        ("render_chart", LocalStateEffect::None),
        ("contract_search", LocalStateEffect::Read),
        ("contract_read", LocalStateEffect::Read),
    ];
    for (name, want) in expected {
        let tool = tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} must be registered"));
        assert_eq!(
            tool.effect.local_state, want,
            "{name} must declare local_state == {want:?}"
        );
    }
    // With writes off, contract_propose is hidden, so none writes local state.
    assert!(
        !tools
            .iter()
            .any(|tool| tool.effect.local_state == LocalStateEffect::WriteCandidate),
        "no tool may declare WriteCandidate when writes are not permitted"
    );

    // No agent tool writes local state any more. Phase F retired
    // `contract_propose`: the harness extracts proposals post-turn from a bounded
    // turn record, so learning no longer depends on the model volunteering a call.
    // Asserting the tool is *absent* is the point — if it reappears, two paths to
    // the same write exist again and the model has to choose between them.
    let tools = DatabaseTools::definitions(true, true, true);
    assert!(
        !tools.iter().any(|tool| tool.name == "contract_propose"),
        "contract_propose is retired; the harness owns proposals"
    );
    assert!(
        !tools
            .iter()
            .any(|tool| tool.effect.local_state == LocalStateEffect::WriteCandidate),
        "no agent tool writes local state once the harness owns extraction"
    );
}

#[test]
fn tool_call_detail_surfaces_the_sql() {
    // Single-connection query: the SQL, whitespace collapsed to one line.
    let detail = tool_call_detail(
        "bounded_sql_query",
        &serde_json::json!({"sql": "SELECT *\n  FROM   users"}),
    )
    .expect("query tools expose their SQL");
    assert_eq!(detail, "SELECT * FROM users");

    // A named connection is annotated.
    let detail = tool_call_detail(
        "bounded_sql_query",
        &serde_json::json!({"sql": "SELECT 1", "connection": "warehouse"}),
    )
    .unwrap();
    assert!(
        detail.contains("SELECT 1") && detail.contains("@warehouse"),
        "got: {detail}"
    );

    // The fan-out tool labels itself as running everywhere.
    let detail = tool_call_detail(
        "bounded_sql_query_all",
        &serde_json::json!({"sql": "SELECT 1"}),
    )
    .unwrap();
    assert!(detail.contains("all connected databases"), "got: {detail}");

    // result_shape is a SQL tool like bounded_sql_query, so its SQL surfaces too.
    let detail = tool_call_detail(
        "result_shape",
        &serde_json::json!({"sql": "SELECT 1", "connection": "warehouse"}),
    )
    .unwrap();
    assert!(
        detail.contains("SELECT 1") && detail.contains("@warehouse"),
        "got: {detail}"
    );

    // Tools without a query expose no detail.
    assert!(tool_call_detail("schema_discovery", &serde_json::json!({})).is_none());
}

#[tokio::test]
async fn test_database_tools_multi_connection_routing() {
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(FakeConnector {
                table_name: "from_primary".into(),
            }),
            dialect: SqlDialect::DuckDb,
            profile_id: None,
        },
    );
    registry.insert(
        "warehouse",
        ConnectionEntry {
            connector: Box::new(FakeConnector {
                table_name: "from_secondary".into(),
            }),
            dialect: SqlDialect::Postgres,
            profile_id: None,
        },
    );

    let tools = DatabaseTools::with_registry(registry, 100, true, None);

    let res = tools
        .execute("schema_discovery", serde_json::json!({}))
        .await
        .expect("schema discovery on primary should succeed");
    assert!(
        res.to_string().contains("from_primary"),
        "expected result to contain 'from_primary', got: {res}"
    );

    let res = tools
        .execute(
            "schema_discovery",
            serde_json::json!({"connection": "warehouse"}),
        )
        .await
        .expect("schema discovery on warehouse should succeed");
    assert!(
        res.to_string().contains("from_secondary"),
        "expected result to contain 'from_secondary', got: {res}"
    );

    let err = tools
        .execute(
            "schema_discovery",
            serde_json::json!({"connection": "nope"}),
        )
        .await
        .expect_err("unknown connection should return error");
    let err_str = err.to_string();
    assert!(
        err_str.contains("primary") && err_str.contains("warehouse"),
        "error message should list available connections, got: {err_str}"
    );
}
