use futures_util::stream::{FuturesUnordered, StreamExt};
use saya_agent::{ToolError, ToolExecutionContext};

use super::DatabaseTools;

impl DatabaseTools {
    /// Runs `sql` against every connected database independently, collecting a
    /// per-database `result` or `error` so a dialect mismatch on one database
    /// never sinks the rest. A single approval covers the whole fan-out.
    pub(super) async fn query_all(
        &self,
        sql: &str,
        context: &ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        let entries = self.registry.operation_entries();
        if entries.is_empty() {
            return Err(ToolError::NoConnectionSelected);
        }
        // A1: detect a contradiction once for the statement, before fanning it
        // out — the same SQL runs against every connection, so per-connection
        // detection would repeat the same finding. The first entry's dialect
        // parses it; if that dialect cannot, the detector fails closed (no
        // finding), consistent with its single-statement behaviour.
        self.detect_and_record_overrides(sql, entries[0].1.entry.dialect);
        let mut entries = entries.into_iter().enumerate();
        let mut pending = FuturesUnordered::new();
        for _ in 0..self.max_concurrent_fan_out_queries {
            if let Some((index, (name, entry))) = entries.next() {
                pending.push(self.query_one(index, name, entry, sql, context));
            }
        }

        let mut databases = Vec::new();
        while let Some(database) = pending.next().await {
            databases.push(database);
            if !context.cancellation.is_cancelled()
                && let Some((index, (name, entry))) = entries.next()
            {
                pending.push(self.query_one(index, name, entry, sql, context));
            }
        }
        databases.sort_by_key(|(index, _, _, _)| *index);

        let databases = databases
            .into_iter()
            .map(|(_, name, dialect, outcome)| {
                let mut record = serde_json::Map::new();
                record.insert("connection".into(), serde_json::Value::String(name));
                record.insert("dialect".into(), serde_json::Value::String(dialect));
                match outcome {
                    Ok(result) => {
                        record.insert("result".into(), result);
                    }
                    Err(error) => {
                        record.insert("error".into(), serde_json::Value::String(error.to_string()));
                    }
                }
                serde_json::Value::Object(record)
            })
            .collect::<Vec<_>>();
        if context.cancellation.is_cancelled() {
            return Err(ToolError::QueryFailedDetail("query cancelled".into()));
        }
        Ok(serde_json::json!({ "databases": databases }))
    }

    async fn query_one(
        &self,
        index: usize,
        name: &str,
        entry: crate::connection::OperationEntry<'_>,
        sql: &str,
        context: &ToolExecutionContext,
    ) -> (usize, String, String, Result<serde_json::Value, ToolError>) {
        let outcome = crate::agent::state_tools::query_with_context(
            &entry,
            sql,
            self.max_rows,
            self.state_db.as_ref(),
            entry.entry.profile_id.as_deref(),
            context,
            Some(self.fan_out_query_timeout),
        )
        .await;
        // Record one observation per connection: a fan-out that touched five
        // databases is five observations, each with its own profile (spec §3).
        if let Some(log) = &self.observations {
            log.record_query(
                "bounded_sql_query_all",
                sql,
                entry.entry.dialect,
                entry.entry.profile_id.as_deref(),
                outcome.as_ref().ok(),
            );
        }
        (
            index,
            name.to_string(),
            entry.entry.dialect.as_str().to_string(),
            outcome,
        )
    }
}
