use super::{compact_schema, query, schema};
use async_trait::async_trait;
use saya_connectors::DatabaseConnector;
use saya_store::{AuditOperation, AuditStore, SchemaStore, SqliteStateStore};
use saya_types::{
    Column, ConnectionError, Database, QueryRequest, QueryResult, Schema, SchemaTree, SqlDialect,
    Table,
};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

struct Failing;
#[async_trait]
impl DatabaseConnector for Failing {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Err(ConnectionError::schema_failed("server sentinel"))
    }
    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Err(ConnectionError::query_failed("row sentinel"))
    }
}

#[tokio::test]
async fn cached_schema_is_explicit_and_agent_query_audit_omits_sql() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("saya-agent-state-{stamp}"));
    let path = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&path);
    let profile = "quoted profile";
    let key = crate::profile_identity::profile_identity(
        profile,
        &saya_types::DatabaseProfile::DuckDb {
            path: "agent-state.duckdb".into(),
            read_only: Some(true),
        },
        std::path::Path::new("/agent-test/connections.toml"),
    )
    .as_str()
    .to_owned();
    store
        .upsert_schema(&key, &SchemaTree::default())
        .await
        .unwrap();
    let cached = schema(&Failing, Some(&store), Some(&key)).await.unwrap();
    assert_eq!(
        cached["diagnostic"],
        "using cached schema because live refresh failed: schema discovery failed: server sentinel"
    );
    assert!(
        query(
            &Failing,
            "SELECT 'raw SQL sentinel'",
            1,
            Some(&store),
            Some(&key)
        )
        .await
        .is_err()
    );
    let audit = store.recent_audit(10).await.unwrap();
    assert!(
        audit
            .iter()
            .any(|row| row.event.operation == AuditOperation::AgentQuery)
    );
    assert!(!String::from_utf8_lossy(&fs::read(&path).unwrap()).contains("raw SQL sentinel"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn compact_schema_qualifies_keys_and_prevents_collisions() {
    let schema_tree = SchemaTree {
        databases: vec![Database {
            name: "main".to_string(),
            schemas: vec![
                Schema {
                    name: "public".to_string(),
                    tables: vec![Table {
                        name: "users".to_string(),
                        columns: vec![Column {
                            name: "id".to_string(),
                            data_type: "INTEGER".to_string(),
                            nullable: false,
                        }],
                    }],
                },
                Schema {
                    name: "sales".to_string(),
                    tables: vec![Table {
                        name: "users".to_string(),
                        columns: vec![Column {
                            name: "email".to_string(),
                            data_type: "TEXT".to_string(),
                            nullable: true,
                        }],
                    }],
                },
            ],
        }],
    };

    let compact = compact_schema(&schema_tree);
    let tables = compact.get("tables").unwrap().as_object().unwrap();

    assert_eq!(tables.len(), 2);
    assert_eq!(tables.get("main.public.users").unwrap(), "id:INTEGER");
    assert_eq!(tables.get("main.sales.users").unwrap(), "email:TEXT");
}

#[test]
fn compact_schema_single_table_uses_fully_qualified_key() {
    let schema_tree = SchemaTree {
        databases: vec![Database {
            name: "db1".to_string(),
            schemas: vec![Schema {
                name: "schema1".to_string(),
                tables: vec![Table {
                    name: "orders".to_string(),
                    columns: vec![
                        Column {
                            name: "id".to_string(),
                            data_type: "INT".to_string(),
                            nullable: false,
                        },
                        Column {
                            name: "amount".to_string(),
                            data_type: "NUMERIC".to_string(),
                            nullable: true,
                        },
                    ],
                }],
            }],
        }],
    };

    let compact = compact_schema(&schema_tree);
    let tables = compact.get("tables").unwrap().as_object().unwrap();

    assert_eq!(tables.len(), 1);
    assert_eq!(
        tables.get("db1.schema1.orders").unwrap(),
        "id:INT, amount:NUMERIC"
    );
}
