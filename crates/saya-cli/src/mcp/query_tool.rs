//! The `query` tool (task Db): one bounded, read-only SQL statement through
//! the connector's safety gate — the same `execute` path `saya query` takes,
//! with the same configured row cap — returned with execution evidence
//! naming this server as the source. A cancelled call stops waiting; where
//! the connector supports cancellation, it is attempted before the unsent
//! error is dropped.

use std::time::Instant;

use rmcp::{
    RoleServer,
    model::{CallToolRequestParams, CallToolResponse, CallToolResult},
    service::RequestContext,
};
use saya_connectors::DatabaseConnector;
use saya_store::AuditStatus;
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryRequest, QueryResult,
};

use super::{
    connector::{audit, build_connector, unix_now_ms},
    context::McpContext,
    policy::ServePolicy,
    tools,
};

pub(crate) async fn query(
    policy: &ServePolicy,
    context: &McpContext,
    request: &CallToolRequestParams,
    request_context: &RequestContext<RoleServer>,
) -> Result<CallToolResponse, rmcp::ErrorData> {
    let name = tools::required_string(request, "profile")?;
    let sql = tools::required_string(request, "sql")?;
    let (profile, identity) = match context.allowed_profile(policy.allowlist(), name) {
        Ok(pair) => pair,
        Err(message) => return Ok(tools::error_result(message)),
    };
    let connector = match build_connector(&profile, context).await {
        Ok(connector) => connector,
        Err(message) => return Ok(tools::error_result(message)),
    };
    if let Err(error) = connector.connect().await {
        return Ok(tools::error_result(error.to_string()));
    }
    let started = Instant::now();
    let started_unix_ms = unix_now_ms();
    let max_rows = context.runtime.resolved.max_rows;
    let execution = connector.execute(QueryRequest::new(sql, max_rows));
    let outcome = tokio::select! {
        _ = request_context.ct.cancelled() => {
            let _ = connector.cancel().await;
            return Ok(tools::error_result("the call was cancelled"));
        }
        outcome = execution => outcome,
    };
    match outcome {
        Ok(result) => {
            audit(
                &context.store,
                identity.as_str(),
                AuditStatus::Success,
                &started,
                Some(result.rows.len()),
                Some(result.truncated),
            )
            .await;
            bounded_payload(
                policy,
                name,
                connector.as_ref(),
                &result,
                max_rows,
                started_unix_ms,
            )
        }
        Err(error) => {
            audit(
                &context.store,
                identity.as_str(),
                AuditStatus::Failure,
                &started,
                None,
                None,
            )
            .await;
            Ok(tools::error_result(error.to_string()))
        }
    }
}

/// The query answer: columns, rows, counts, and evidence. A result over the
/// response bound is narrowed by dropping whole rows — measured on the FINAL
/// built wire result, the narrowing note included, never the payload alone
/// (A922-5, D5) — never sent fat. The narrowed answer stays internally
/// consistent: `row_count` is the row count sent, the note names the cut,
/// and the evidence copy reports the sent rows with `truncated: true`; the
/// un-narrowed execution facts stay in the audit. Only when even the
/// rowless result exceeds the bound is the answer refused.
fn bounded_payload(
    policy: &ServePolicy,
    name: &str,
    connector: &dyn DatabaseConnector,
    result: &QueryResult,
    max_rows: usize,
    started_unix_ms: i64,
) -> Result<CallToolResponse, rmcp::ErrorData> {
    let mut evidence = ExecutionEvidence::for_result(
        result,
        ExecutionEvidenceArgs {
            execution_id: ExecutionEvidence::new_execution_id(started_unix_ms, 0),
            connection_label: name.to_owned(),
            connection_identity: None,
            dialect: connector.dialect(),
            max_rows,
            started_unix_ms,
            finished_unix_ms: unix_now_ms(),
            source: EvidenceSource::Mcp,
        },
    );
    let mut rows = result.rows.clone();
    let total = result.row_count;
    let mut truncated = result.truncated;
    loop {
        let mut payload = serde_json::json!({
            "columns": result.columns,
            "rows": rows,
            "row_count": rows.len(),
            "truncated": truncated,
            "evidence": evidence,
        });
        // The narrowing note rides the measured payload: the answer that is
        // sent is exactly the answer that was measured.
        if truncated != result.truncated {
            payload["note"] = serde_json::Value::String(format!(
                "the response exceeded the byte bound and was cut to {}/{} rows",
                rows.len(),
                total
            ));
        }
        let built = CallToolResult::structured(payload);
        if policy.response_allowed(tools::result_wire_bytes(&built)) || rows.is_empty() {
            return tools::bounded_built_result(policy, built);
        }
        rows.truncate(rows.len() / 2);
        truncated = true;
        evidence.returned_rows = rows.len();
        evidence.truncated = true;
    }
}
