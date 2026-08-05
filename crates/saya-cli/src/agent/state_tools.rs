use saya_connectors::DatabaseConnector;
use saya_store::{
    AuditEntry, AuditOperation, AuditStatus, AuditStore, SchemaStore, SqliteStateStore,
};
use saya_types::{QueryRequest, SchemaTree};
use std::time::Instant;

/// Rows returned to the MODEL from a tool call are capped small: the model
/// reasons over a sample and should use aggregate SQL for counts, so feeding it
/// hundreds of rows only bloats context and slows every later turn.
const MODEL_ROW_CAP: usize = 50;

/// Flattens a schema into a compact `{ "tables": { name: "col:type, ..." } }`
/// map — far smaller than the full serialized tree, which otherwise rides in
/// context on every subsequent agent turn.
fn compact_schema(schema: &SchemaTree) -> serde_json::Value {
    let mut tables = serde_json::Map::new();
    for database in &schema.databases {
        for schema_ns in &database.schemas {
            for table in &schema_ns.tables {
                let columns = table
                    .columns
                    .iter()
                    .map(|column| format!("{}:{}", column.name, column.data_type))
                    .collect::<Vec<_>>()
                    .join(", ");
                tables.insert(table.name.clone(), serde_json::Value::String(columns));
            }
        }
    }
    serde_json::json!({ "tables": tables })
}

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
            Ok(compact_schema(&schema))
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
            let mut value = compact_schema(&cached.schema);
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "diagnostic".into(),
                    serde_json::Value::String("cached schema may be stale".into()),
                );
            }
            Ok(value)
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
    // Cap the rows the MODEL sees (not the /sql display path, which keeps max_rows).
    let model_rows = max_rows.min(MODEL_ROW_CAP);
    match connector.execute(QueryRequest::new(sql, model_rows)).await {
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
