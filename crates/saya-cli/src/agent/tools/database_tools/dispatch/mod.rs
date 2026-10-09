//! Dispatches a read-only agent tool call to its connection and records an
//! observation of what it touched. The recording lives here, beside the
//! dispatch, because this is the only place that knows the tool name, the SQL,
//! the target connection, and the result together (spec §3).

mod data_tools;

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
        context: saya_agent::ToolExecutionContext,
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
        // The clarification ask (B3c) resolves after argument validation and
        // before connection resolution: it touches no connection, no
        // database, and records nothing — the loop validates the bounds and
        // ends the turn on the call, and this arm answers a directly-reached
        // call with the short result the model receives. Like the contract
        // tools it records NOTHING: an ask is not evidence about a database
        // object.
        if name == "request_clarification" {
            return Ok(serde_json::json!({
                "asked": true,
                "next": "stop your turn here; the user will answer in their next message"
            }));
        }
        // The workspace tools are dispatched before connection resolution:
        // they read or write the run's workspace directory, not a database,
        // and a workspace-only run has no selected profile for
        // `registry.resolve` to fail on. Like the contract tools they record
        // NOTHING — an observation about a workspace file or path is not
        // evidence about a database object. (`workspace_write` reaches this
        // routing only when the loop's permit gate already allowed it; the
        // routing decides where it runs, never whether.)
        if super::super::workspace::is_tool_name(name) {
            return super::super::workspace::execute_tool(self, name, &arguments).await;
        }
        if matches!(
            name,
            "bounded_sql_query"
                | "bounded_sql_query_all"
                | "render_chart"
                | "result_shape"
                | "column_health"
                | "join_check"
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
            return self.query_all(sql, &context).await;
        }
        let connection = arguments
            .get("connection")
            .and_then(serde_json::Value::as_str);
        let mut operation = self.registry.resolve_for_operation(connection)?;
        operation.metrics = self.metrics.as_deref();
        self.dispatch_data_tool(name, arguments, context, operation)
            .await
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
