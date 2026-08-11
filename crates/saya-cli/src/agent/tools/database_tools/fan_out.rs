use futures_util::stream::{FuturesUnordered, StreamExt};

use super::DatabaseTools;

impl DatabaseTools {
    /// Runs `sql` against every connected database independently, collecting a
    /// per-database `result` or `error` so a dialect mismatch on one database
    /// never sinks the rest. A single approval covers the whole fan-out.
    pub(super) async fn query_all(&self, sql: &str) -> Result<serde_json::Value, String> {
        let entries = self.registry.entries();
        if entries.is_empty() {
            return Err("no database profile is selected".into());
        }
        let mut entries = entries.into_iter().enumerate();
        let mut pending = FuturesUnordered::new();
        for _ in 0..self.max_concurrent_fan_out_queries {
            if let Some((index, (name, entry))) = entries.next() {
                pending.push(self.query_one(index, name, entry, sql));
            }
        }

        let mut databases = Vec::new();
        while let Some(database) = pending.next().await {
            databases.push(database);
            if let Some((index, (name, entry))) = entries.next() {
                pending.push(self.query_one(index, name, entry, sql));
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
                        record.insert("error".into(), serde_json::Value::String(error));
                    }
                }
                serde_json::Value::Object(record)
            })
            .collect::<Vec<_>>();
        Ok(serde_json::json!({ "databases": databases }))
    }

    async fn query_one(
        &self,
        index: usize,
        name: &str,
        entry: &crate::connection::ConnectionEntry,
        sql: &str,
    ) -> (usize, String, String, Result<serde_json::Value, String>) {
        let outcome = tokio::time::timeout(
            self.fan_out_query_timeout,
            crate::agent::state_tools::query(
                entry.connector.as_ref(),
                sql,
                self.max_rows,
                self.state_db.as_ref(),
                entry.profile_id.as_deref(),
            ),
        )
        .await
        .unwrap_or_else(|_| Err("read-only query timed out".into()));
        (
            index,
            name.to_string(),
            entry.dialect.as_str().to_string(),
            outcome,
        )
    }
}
