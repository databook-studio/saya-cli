use saya_agent::ToolError;
use saya_connectors::DatabaseConnector;
use saya_store::{
    AuditEntry, AuditOperation, AuditStatus, AuditStore, SchemaStore, SqliteStateStore,
};
use saya_types::{ForeignKey, QueryRequest, SchemaTree, SqlDialect};
use std::collections::HashMap;
use std::time::Instant;

/// Rows returned to the MODEL from a tool call are capped small: the model
/// reasons over a sample and should use aggregate SQL for counts, so feeding it
/// hundreds of rows only bloats context and slows every later turn.
const MODEL_ROW_CAP: usize = 50;

/// One table's worth of rendered schema, gathered before keys are resolved so
/// a foreign key can find its target anywhere in the tree.
struct CompactEntry {
    database: String,
    schema: String,
    name: String,
    short: String,
    full: String,
    columns: String,
    primary_key: Vec<String>,
    foreign_keys: Vec<ForeignKey>,
}

/// Flattens a schema into a compact `{ "tables": { name: "..." } }` map —
/// far smaller than the full serialized tree, which otherwise rides in
/// context on every subsequent agent turn.
/// The schema as the model sees it: one entry per table, keyed by the name it
/// should write in SQL.
///
/// The key is an example the model copies, so its depth follows the engine
/// rather than the tree. Every engine is discovered as catalog → schema →
/// table, but SQLite parses only the table name and MySQL only
/// `database.table`; keying those three-deep hands the model a name its own
/// engine rejects, costing a failed statement before it retries. Where the
/// engine's depth cannot separate two tables, both keep the full name — a name
/// that needs correcting beats a table silently missing from the schema.
///
/// Each value is the columns (`name:type`, comma-separated) followed by the
/// primary key and foreign keys only when present, so a table with neither
/// serializes exactly as before. Foreign keys name their target by the same
/// key it appears under elsewhere in this map, so the model can resolve a
/// join target without guessing which same-named table is meant.
fn compact_schema(schema: &SchemaTree, dialect: SqlDialect) -> serde_json::Value {
    let mut entries: Vec<CompactEntry> = Vec::new();
    for database in &schema.databases {
        for schema_ns in &database.schemas {
            for table in &schema_ns.tables {
                let full = format!("{}.{}.{}", database.name, schema_ns.name, table.name);
                let short = match dialect.sql_name_parts() {
                    1 => table.name.clone(),
                    2 => format!("{}.{}", database.name, table.name),
                    _ => full.clone(),
                };
                let columns = table
                    .columns
                    .iter()
                    .map(|column| format!("{}:{}", column.name, column.data_type))
                    .collect::<Vec<_>>()
                    .join(", ");
                entries.push(CompactEntry {
                    database: database.name.clone(),
                    schema: schema_ns.name.clone(),
                    name: table.name.clone(),
                    short,
                    full,
                    columns,
                    primary_key: table.primary_key.clone(),
                    foreign_keys: table.foreign_keys.clone(),
                });
            }
        }
    }

    let key_of = |entry: &CompactEntry| -> String {
        let ambiguous = entries
            .iter()
            .filter(|other| other.short == entry.short)
            .count()
            > 1;
        if ambiguous {
            entry.full.clone()
        } else {
            entry.short.clone()
        }
    };
    let keys: Vec<String> = entries.iter().map(key_of).collect();

    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        by_name.entry(entry.name.as_str()).or_default().push(index);
    }

    let mut tables = serde_json::Map::new();
    for (index, entry) in entries.iter().enumerate() {
        let mut value = entry.columns.clone();
        if !entry.primary_key.is_empty() {
            value.push_str(" | pk=");
            value.push_str(&entry.primary_key.join(","));
        }
        if !entry.foreign_keys.is_empty() {
            value.push_str(" | fk=");
            let rendered: Vec<String> = entry
                .foreign_keys
                .iter()
                .map(|fk| {
                    let referenced_key =
                        resolve_referenced_key(fk, index, &entries, &keys, &by_name);
                    format!(
                        "{}->{}.{}",
                        column_list(&fk.columns),
                        referenced_key,
                        column_list(&fk.referenced_columns)
                    )
                })
                .collect();
            value.push_str(&rendered.join(";"));
        }
        tables.insert(keys[index].clone(), serde_json::Value::String(value));
    }

    serde_json::json!({ "tables": tables })
}

/// One column renders bare; several render as a parenthesised, comma-separated
/// list so a composite key's column order stays visible and pairable.
fn column_list(columns: &[String]) -> String {
    if columns.len() == 1 {
        columns[0].clone()
    } else {
        format!("({})", columns.join(","))
    }
}

/// The key the referenced table appears under in the rendered map. Prefers the
/// schema the connector named, then the referencing table's own schema, then
/// any table of that name; falls back to the bare name the connector reported
/// when the target is not in the tree at all.
fn resolve_referenced_key(
    fk: &ForeignKey,
    referencing_index: usize,
    entries: &[CompactEntry],
    keys: &[String],
    by_name: &HashMap<&str, Vec<usize>>,
) -> String {
    let candidates = by_name
        .get(fk.referenced_table.as_str())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let referencing = &entries[referencing_index];
    let chosen = match &fk.referenced_schema {
        Some(schema) => candidates
            .iter()
            .copied()
            .find(|&i| entries[i].schema == *schema),
        None => candidates
            .iter()
            .copied()
            .find(|&i| {
                entries[i].database == referencing.database
                    && entries[i].schema == referencing.schema
            })
            .or_else(|| {
                candidates
                    .iter()
                    .copied()
                    .find(|&i| entries[i].database == referencing.database)
            }),
    };
    chosen
        .map(|i| keys[i].clone())
        .unwrap_or_else(|| fk.referenced_table.clone())
}

pub(crate) async fn schema(
    connector: &dyn DatabaseConnector,
    store: Option<&SqliteStateStore>,
    profile_id: Option<&str>,
) -> Result<serde_json::Value, ToolError> {
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
            Ok(compact_schema(&schema, connector.dialect()))
        }
        Err(error) => cached(store, profile_id, started, &error, connector.dialect()).await,
    }
}

async fn cached(
    store: Option<&SqliteStateStore>,
    profile_id: Option<&str>,
    started: Instant,
    live_error: &saya_types::ConnectionError,
    dialect: SqlDialect,
) -> Result<serde_json::Value, ToolError> {
    let (Some(store), Some(profile_id)) = (store, profile_id) else {
        return Err(ToolError::SchemaDiscoveryFailed(live_error.to_string()));
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
            let mut value = compact_schema(&cached.schema, dialect);
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "diagnostic".into(),
                    serde_json::Value::String(format!(
                        "using cached schema because live refresh failed: {live_error}"
                    )),
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
            Err(ToolError::SchemaDiscoveryFailed(live_error.to_string()))
        }
    }
}

pub(crate) async fn query(
    connector: &dyn DatabaseConnector,
    sql: &str,
    max_rows: usize,
    store: Option<&SqliteStateStore>,
    profile_id: Option<&str>,
) -> Result<serde_json::Value, ToolError> {
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
            serde_json::to_value(result).map_err(|_| ToolError::QueryResultUnavailable)
        }
        Err(error) => {
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
            // Connector errors are intentionally sanitized at their boundary, so
            // their safe detail helps the agent distinguish dialect and syntax
            // mismatches without exposing driver internals or credentials.
            Err(ToolError::QueryFailedDetail(error.to_string()))
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
