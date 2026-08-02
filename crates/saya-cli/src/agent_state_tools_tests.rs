use super::{query, schema};
use async_trait::async_trait;
use saya_connectors::DatabaseConnector;
use saya_store::{AuditOperation, AuditStore, SchemaStore, SqliteStateStore};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
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
        Err(ConnectionError::SchemaFailed("server sentinel".into()))
    }
    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Err(ConnectionError::QueryFailed("row sentinel".into()))
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
    let key = crate::profile_identity::profile_identity(profile);
    store
        .upsert_schema(&key, &SchemaTree::default())
        .await
        .unwrap();
    let cached = schema(&Failing, Some(&store), Some(profile)).await.unwrap();
    assert_eq!(cached["diagnostic"], "cached schema may be stale");
    assert!(
        query(
            &Failing,
            "SELECT 'raw SQL sentinel'",
            1,
            Some(&store),
            Some(profile)
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
