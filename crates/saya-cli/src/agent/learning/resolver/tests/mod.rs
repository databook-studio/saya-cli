use super::*;
use crate::connection::ConnectionEntry;
use async_trait::async_trait;
use saya_connectors::DatabaseConnector;
use saya_types::{
    Column, ColumnRequirement, ColumnRole, ConnectionError, Database, KnowledgeSlot,
    KnowledgeState, ProfileIdentity, QueryRequest, QueryResult, Schema, SchemaTree, SqlDialect,
    Table,
};

struct TestConnector {
    tree: SchemaTree,
}

#[async_trait]
impl DatabaseConnector for TestConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(self.tree.clone())
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult {
            columns: vec!["id".into()],
            rows: vec![serde_json::json!([1])],
            row_count: 1,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

fn sample_tree() -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![
                    Table {
                        name: "orders".into(),
                        primary_key: vec![],
                        foreign_keys: vec![],
                        columns: vec![
                            Column {
                                name: "id".into(),
                                data_type: "bigint".into(),
                                nullable: false,
                            },
                            Column {
                                name: "created_at".into(),
                                data_type: "timestamp".into(),
                                nullable: false,
                            },
                        ],
                    },
                    Table {
                        name: "staff".into(),
                        primary_key: vec![],
                        foreign_keys: vec![],
                        columns: vec![
                            Column {
                                name: "id".into(),
                                data_type: "bigint".into(),
                                nullable: false,
                            },
                            Column {
                                name: "name".into(),
                                data_type: "text".into(),
                                nullable: false,
                            },
                        ],
                    },
                ],
            }],
        }],
    }
}

fn test_registry(name: &str, identity: &ProfileIdentity, tree: SchemaTree) -> ConnectionRegistry {
    let mut reg = ConnectionRegistry::new(name);
    reg.insert(
        name,
        ConnectionEntry {
            connector: Box::new(TestConnector { tree }),
            dialect: SqlDialect::DuckDb,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    reg
}

fn test_identity() -> ProfileIdentity {
    ProfileIdentity::parse("p-1111111111111111111111111111111111111111111111111111111111111111")
        .unwrap()
}

mod columns;
mod origins;
mod proposal_binding;
mod qualification;
mod turn_identity;
