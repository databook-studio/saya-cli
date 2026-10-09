//! The connector-backed tool arms of the dispatch: run the selected
//! operation and record the observation of what it touched (spec §3).

use super::DatabaseTools;
use crate::connection::OperationEntry;
use saya_agent::ToolError;

impl DatabaseTools {
    pub(super) async fn dispatch_data_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: saya_agent::ToolExecutionContext,
        operation: OperationEntry<'_>,
    ) -> Result<serde_json::Value, ToolError> {
        let entry = operation.entry;
        match name {
            "schema_discovery" => {
                let profile_id = entry.profile_id.as_deref();
                let result = crate::agent::state_tools::schema_with_context(
                    &operation,
                    &context,
                    self.state_db.as_ref(),
                    profile_id,
                )
                .await;
                reject_cancelled(&context)?;
                if let Some(log) = &self.observations {
                    log.record_schema(name, profile_id, result.is_ok());
                }
                result
            }
            "bounded_sql_query" => {
                let sql = arguments
                    .get("sql")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(ToolError::InvalidQueryArguments)?;
                // A1: detect a confirmed claim this statement contradicts, from
                // the statement itself — independent of whether the query then
                // succeeds (the override is about the statement the model wrote).
                // Best-effort: a missing receipt/log or a fail-closed detector
                // records nothing; the turn is never failed by detection.
                self.detect_and_record_overrides(sql, entry.dialect);
                let executed = crate::agent::state_tools::query_typed(
                    &operation,
                    sql,
                    self.max_rows,
                    self.state_db.as_ref(),
                    entry.profile_id.as_deref(),
                    &context,
                    None,
                )
                .await;
                reject_cancelled(&context)?;
                if let Some(log) = &self.observations {
                    log.record_query(
                        name,
                        sql,
                        entry.dialect,
                        entry.profile_id.as_deref(),
                        executed.as_ref().ok().map(|ok| &ok.value),
                    );
                }
                // The capture hook fires before this returns, so the typed
                // result the model saw reaches the TUI before the loop's
                // ToolCompleted for the same call. Only here, only on Ok —
                // never fan-out or the probe tools.
                if let Ok(executed) = &executed {
                    self.emit_query_capture(&arguments, sql, entry, executed, &context);
                }
                executed.map(|executed| executed.value)
            }
            "result_shape" => {
                let sql = arguments
                    .get("sql")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(ToolError::InvalidQueryArguments)?;
                let result = self.result_shape(&operation, sql, &context).await;
                reject_cancelled(&context)?;
                if let Some(log) = &self.observations {
                    // result_shape runs SQL but returns no rows; record it like
                    // a query so the objects it touched are still credited as
                    // evidence, reading row_count/truncated from the shape when
                    // it succeeded.
                    log.record_query(
                        name,
                        sql,
                        entry.dialect,
                        entry.profile_id.as_deref(),
                        result.as_ref().ok(),
                    );
                }
                result
            }
            "column_health" => {
                let sql = arguments
                    .get("sql")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(ToolError::InvalidQueryArguments)?;
                let result = self.column_health(&operation, sql, &context).await;
                reject_cancelled(&context)?;
                if let Some(log) = &self.observations {
                    log.record_query(
                        name,
                        sql,
                        entry.dialect,
                        entry.profile_id.as_deref(),
                        result.as_ref().ok(),
                    );
                }
                result
            }
            "join_check" => {
                let sql = arguments
                    .get("sql")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(ToolError::InvalidQueryArguments)?;
                let result = self.join_check(&operation, sql, &context).await;
                reject_cancelled(&context)?;
                if let Some(log) = &self.observations {
                    log.record_query(
                        name,
                        sql,
                        entry.dialect,
                        entry.profile_id.as_deref(),
                        result.as_ref().ok(),
                    );
                }
                result
            }
            "render_chart" => {
                let sql = arguments
                    .get("sql")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(ToolError::InvalidQueryArguments)?;
                let result = self.render_chart(&arguments, &operation, &context).await;
                reject_cancelled(&context)?;
                if let Some(log) = &self.observations {
                    // render_chart runs SQL but returns a path, not rows; it has
                    // no row_count/truncated to record, so it records like a
                    // query with only the objects/columns it named.
                    log.record_query(
                        name,
                        sql,
                        entry.dialect,
                        entry.profile_id.as_deref(),
                        result.as_ref().ok(),
                    );
                }
                result
            }
            _ => Err(ToolError::UnsupportedTool),
        }
    }
}

fn reject_cancelled(context: &saya_agent::ToolExecutionContext) -> Result<(), ToolError> {
    if context.cancellation.is_cancelled() {
        Err(ToolError::QueryFailedDetail("query cancelled".into()))
    } else {
        Ok(())
    }
}
