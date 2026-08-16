//! Dispatches a read-only agent tool call to its connection and records an
//! observation of what it touched. The recording lives here, beside the
//! dispatch, because this is the only place that knows the tool name, the SQL,
//! the target connection, and the result together (spec §3).

use saya_agent::ToolError;

use super::DatabaseTools;
use super::definitions::validate_arguments;
use super::observations::{ObservationOutcome, ToolObservation};

impl DatabaseTools {
    /// Dispatches a read-only agent tool call to its selected connection.
    //
    // `pub(in crate::agent::tools)` so the sibling `executor` module (which
    // implements `ToolExecutor for DatabaseTools`) can call it; this matches the
    // visibility the method had when it lived directly in `mod.rs`.
    pub(in crate::agent::tools) async fn execute_read_only(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        // Contract tools have their own argument validation and execution
        // (sibling concern) and never reach a connector; route them before the
        // database-tool validation, which would reject their names.
        //
        // Contract tool calls record NOTHING. An observation about reading
        // memory is not evidence about a database object, and recording it would
        // let memory reinforce itself (spec §3). This early return is the whole
        // of that rule: no recording, ever, for `contract_search`/`contract_read`.
        if matches!(name, "contract_search" | "contract_read") {
            return self.execute_contract_tool(name, arguments).await;
        }
        validate_arguments(name, &arguments)?;
        if matches!(
            name,
            "bounded_sql_query" | "bounded_sql_query_all" | "render_chart"
        ) && !self.allow_query_data
        {
            // The data-sharing gate refuses a query tool before it touches a
            // database. This is the denial reachable from saya-cli: the agent
            // loop's user-approval denial lives in saya-agent and never reaches
            // this executor, so this is the boundary that records `Denied`.
            // A denied call leaves no positive evidence: no objects, no rows.
            self.record_denied(name);
            return Err(ToolError::DataSharingDisabled);
        }
        if name == "bounded_sql_query_all" {
            let sql = arguments
                .get("sql")
                .and_then(serde_json::Value::as_str)
                .ok_or(ToolError::InvalidQueryArguments)?;
            return self.query_all(sql).await;
        }
        let connection = arguments
            .get("connection")
            .and_then(serde_json::Value::as_str);
        let entry = self.registry.resolve(connection)?;
        match name {
            "schema_discovery" => {
                let profile_id = entry.profile_id.as_deref();
                let result = crate::agent::state_tools::schema(
                    entry.connector.as_ref(),
                    self.state_db.as_ref(),
                    profile_id,
                )
                .await;
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
                let result = crate::agent::state_tools::query(
                    entry.connector.as_ref(),
                    sql,
                    self.max_rows,
                    self.state_db.as_ref(),
                    entry.profile_id.as_deref(),
                )
                .await;
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
                let result = self.render_chart(&arguments).await;
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

    fn record_denied(&self, name: &str) {
        let Some(log) = &self.observations else {
            return;
        };
        log.record(ToolObservation {
            tool: name.into(),
            outcome: ObservationOutcome::Denied,
            profile: None,
            objects: Vec::new(),
            columns: Vec::new(),
            row_count: None,
            truncated: None,
            references_partial: false,
        });
    }
}
