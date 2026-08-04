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
