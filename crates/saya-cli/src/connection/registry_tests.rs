use super::*;
use async_trait::async_trait;
use saya_connectors::DatabaseConnector;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

struct DummyConnector {
    dialect: SqlDialect,
}

#[async_trait]
impl DatabaseConnector for DummyConnector {
    fn dialect(&self) -> SqlDialect {
        self.dialect
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Err(ConnectionError::SchemaFailed("dummy".into()))
    }

    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Err(ConnectionError::QueryFailed("dummy".into()))
    }
}

fn make_entry(dialect: SqlDialect, profile_id: Option<&str>) -> ConnectionEntry {
    ConnectionEntry {
        connector: Box::new(DummyConnector { dialect }),
        dialect,
        profile_id: profile_id.map(ToString::to_string),
    }
}

#[test]
fn test_resolve_behavior() {
    let mut reg = ConnectionRegistry::new("primary_db");

    // Empty registry resolves to Err("no database profile is selected")
    assert_eq!(
        reg.resolve(None).unwrap_err(),
        "no database profile is selected"
    );
    assert_eq!(
        reg.resolve(Some("")).unwrap_err(),
        "no database profile is selected"
    );
    assert_eq!(
        reg.resolve(Some("any")).unwrap_err(),
        "no database profile is selected"
    );

    // Insert primary and secondary
    reg.insert("primary_db", make_entry(SqlDialect::Postgres, Some("p1")));
    reg.insert("secondary_db", make_entry(SqlDialect::DuckDb, Some("p2")));

    // resolve(None) and resolve(Some("")) return primary entry
    let primary_entry = reg.resolve(None).unwrap();
    assert_eq!(primary_entry.dialect, SqlDialect::Postgres);
    assert_eq!(primary_entry.profile_id.as_deref(), Some("p1"));

    let primary_empty = reg.resolve(Some("")).unwrap();
    assert_eq!(primary_empty.dialect, SqlDialect::Postgres);

    // resolve(Some(known)) returns known secondary entry
    let secondary_entry = reg.resolve(Some("secondary_db")).unwrap();
    assert_eq!(secondary_entry.dialect, SqlDialect::DuckDb);
    assert_eq!(secondary_entry.profile_id.as_deref(), Some("p2"));

    // resolve(Some(unknown)) returns Err listing available connections
    let err = reg.resolve(Some("unknown_db")).unwrap_err();
    assert_eq!(
        err,
        "unknown connection \"unknown_db\"; available connections: primary_db, secondary_db"
    );
}

#[test]
fn test_names_insertion_order() {
    let mut reg = ConnectionRegistry::new("primary_db");
    assert!(reg.is_empty());
    assert_eq!(reg.len(), 0);
    assert_eq!(reg.primary_name(), "primary_db");

    reg.insert("primary_db", make_entry(SqlDialect::Postgres, None));
    reg.insert("alpha", make_entry(SqlDialect::Mysql, None));
    reg.insert("beta", make_entry(SqlDialect::DuckDb, None));

    assert_eq!(reg.len(), 3);
    assert!(!reg.is_empty());
    assert_eq!(reg.names(), vec!["primary_db", "alpha", "beta"]);

    // Re-inserting existing connection updates entry but preserves first-seen order
    reg.insert("alpha", make_entry(SqlDialect::Snowflake, None));
    assert_eq!(reg.names(), vec!["primary_db", "alpha", "beta"]);
    assert_eq!(
        reg.resolve(Some("alpha")).unwrap().dialect,
        SqlDialect::Snowflake
    );
}

#[test]
fn test_describe_context() {
    let mut reg = ConnectionRegistry::new("db1");

    // Empty registry: describe_context is None
    assert!(reg.describe_context().is_none());

    // Single connection: describe_context is None
    reg.insert("db1", make_entry(SqlDialect::Postgres, None));
    assert!(reg.describe_context().is_none());

    // Two connections: describe_context is Some(...) containing both names and dialects
    reg.insert("db2", make_entry(SqlDialect::DuckDb, None));
    let ctx = reg
        .describe_context()
        .expect("context should be Some for >1 connections");

    assert!(ctx.contains("- db1 (postgresql)"));
    assert!(ctx.contains("- db2 (duckdb)"));
    assert!(ctx.contains("connection"));
    assert!(ctx.contains("schema and query tools"));
}
