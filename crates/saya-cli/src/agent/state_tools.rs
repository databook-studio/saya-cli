use saya_connectors::DatabaseConnector;
use saya_store::{
    AuditEntry, AuditOperation, AuditStatus, AuditStore, SchemaStore, SqliteStateStore,
};
use saya_types::QueryRequest;
use std::time::Instant;

pub(crate) async fn schema(
    connector: &dyn DatabaseConnector,
    store: Option<&SqliteStateStore>,
    profile_id: Option<&str>,
) -> Result<serde_json::Value, String> {
    let started = Instant::now();
    match connector.schema().await {
        Ok(schema) => {
            if let (Some(store), Some(profile_id)) = (store, profile_id) {
                let _ = store.upsert_schema(profile_id, &schema).await;
                audit(
                    store,
                    profile_id,
                    AuditOperation::SchemaRefresh,
                    AuditStatus::Success,
                    started,
                    None,
                    None,
                )
                .await;
            }
            serde_json::to_value(schema).map_err(|_| "schema result unavailable".into())
        }
        Err(_) => cached(store, profile_id, started).await,
    }
}

async fn cached(
    store: Option<&SqliteStateStore>,
    profile_id: Option<&str>,
    started: Instant,
) -> Result<serde_json::Value, String> {
    let (Some(store), Some(profile_id)) = (store, profile_id) else {
        return Err("schema discovery failed".into());
    };
    match store.get_schema(profile_id).await {
        Ok(Some(cached)) => {
            audit(
                store,
                profile_id,
                AuditOperation::SchemaRefresh,
                AuditStatus::Cached,
                started,
                None,
                None,
            )
            .await;
            Ok(
                serde_json::json!({"schema": cached.schema, "diagnostic": "cached schema may be stale"}),
            )
        }
        _ => {
            audit(
                store,
                profile_id,
                AuditOperation::SchemaRefresh,
                AuditStatus::Failure,
                started,
                None,
                None,
            )
            .await;
            Err("schema discovery failed".into())
        }
    }
}

pub(crate) async fn query(
    connector: &dyn DatabaseConnector,
    sql: &str,
    max_rows: usize,
    store: Option<&SqliteStateStore>,
    profile_id: Option<&str>,
) -> Result<serde_json::Value, String> {
    let started = Instant::now();
    match connector.execute(QueryRequest::new(sql, max_rows)).await {
        Ok(result) => {
            if let (Some(store), Some(profile_id)) = (store, profile_id) {
                audit(
                    store,
                    profile_id,
                    AuditOperation::AgentQuery,
                    AuditStatus::Success,
                    started,
                    Some(result.rows.len()),
                    Some(result.truncated),
                )
                .await;
            }
            serde_json::to_value(result).map_err(|_| "query result unavailable".into())
        }
        Err(_) => {
            if let (Some(store), Some(profile_id)) = (store, profile_id) {
                audit(
                    store,
                    profile_id,
                    AuditOperation::AgentQuery,
                    AuditStatus::Failure,
                    started,
                    None,
                    None,
                )
                .await;
            }
            Err("read-only query failed".into())
        }
    }
}

async fn audit(
    store: &SqliteStateStore,
    profile_id: &str,
    operation: AuditOperation,
    status: AuditStatus,
    started: Instant,
    rows: Option<usize>,
    truncated: Option<bool>,
) {
    let mut event = AuditEntry::new(
        profile_id,
        operation,
        status,
        started.elapsed().as_millis() as u64,
    );
    event.row_count = rows;
    event.truncated = truncated;
    let _ = store.record_audit(event).await;
}

#[cfg(test)]
#[path = "state_tools_tests.rs"]
mod tests;
