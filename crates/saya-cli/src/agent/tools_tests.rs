use super::*;
use async_trait::async_trait;
use saya_agent::ToolExecutor;
use saya_connectors::DatabaseConnector;
use saya_types::{
    ConnectionError, Database, QueryRequest, QueryResult, Schema, SchemaTree, SqlDialect, Table,
};

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

#[async_trait]
impl DatabaseConnector for FailingConnector {
    fn dialect(&self) -> SqlDialect {
        self.dialect
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Err(ConnectionError::SchemaFailed("nope".into()))
    }

    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Err(ConnectionError::QueryFailed(
            "syntax error near FROM".into(),
        ))
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
    assert!(err.contains("data sharing is disabled"), "got: {err}");
}

#[test]
fn definitions_include_fan_out_only_when_query_data_allowed() {
    let with_data: Vec<String> = DatabaseTools::definitions(true)
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert!(with_data.iter().any(|name| name == "bounded_sql_query_all"));

    let without_data: Vec<String> = DatabaseTools::definitions(false)
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
    assert!(
        err.contains("primary") && err.contains("warehouse"),
        "error message should list available connections, got: {err}"
    );
}
