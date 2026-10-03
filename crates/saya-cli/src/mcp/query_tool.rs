//! The `query` tool (task Db): one bounded, read-only SQL statement through
//! the connector's safety gate — the same `execute` path `saya query` takes,
//! with the same configured row cap — returned with execution evidence
//! naming this server as the source. Cancellation and timeout request
//! connector cancellation but retain the execute future until it settles;
//! the request outcome does not prove the database stopped.

use std::time::Instant;

use crate::interactive::sql_operation::cancellation::SettledQuery;
use rmcp::{
    RoleServer,
    model::{CallToolRequestParams, CallToolResponse, CallToolResult, RequestId},
    service::RequestContext,
};
use saya_agent::CancellationToken;
use saya_connectors::DatabaseConnector;
use saya_store::AuditStatus;
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryRequest, QueryResult,
};

#[cfg(test)]
#[path = "query_tool_tests.rs"]
mod query_tool_tests;

use super::{
    connector::{audit, build_connector, unix_now_ms},
    context::McpContext,
    policy::ServePolicy,
    query_cancellation::{QueryExecution, settle_query_execution},
    tools,
};

pub(crate) async fn query(
    policy: &ServePolicy,
    context: &McpContext,
    request: &CallToolRequestParams,
    request_context: &RequestContext<RoleServer>,
    request_deadline: tokio::time::Instant,
) -> Result<CallToolResponse, rmcp::ErrorData> {
    let name = tools::required_string(request, "profile")?;
    let sql = tools::required_string(request, "sql")?;
    let (profile, identity) = match context.allowed_profile(policy.allowlist(), name) {
        Ok(pair) => pair,
        Err(message) => return Ok(tools::error_result(message)),
    };
    let cancellation = CancellationToken::new();
    let deadline = request_deadline;
    if let Some(result) =
        stopped_before_execute(request_context.ct.is_cancelled(), deadline, &cancellation)
    {
        return Ok(result);
    }
    let connector = match build_connector(&profile, context).await {
        Ok(connector) => connector,
        Err(message) => return Ok(tools::error_result(message)),
    };
    if let Some(result) =
        stopped_before_execute(request_context.ct.is_cancelled(), deadline, &cancellation)
    {
        return Ok(result);
    }
    if let Err(error) = connector.connect().await {
        return Ok(tools::error_result(error.to_string()));
    }
    if let Some(result) =
        stopped_before_execute(request_context.ct.is_cancelled(), deadline, &cancellation)
    {
        return Ok(result);
    }
    let started = Instant::now();
    let started_unix_ms = unix_now_ms();
    let max_rows = context.runtime.resolved.max_rows;
    let outcome = settle_query_execution(
        connector.as_ref(),
        QueryRequest::new(sql, max_rows),
        &cancellation,
        request_context.ct.cancelled(),
        || request_context.ct.is_cancelled(),
        deadline,
    )
    .await;
    let outcome = match outcome {
        QueryExecution::Completed(settled) => settled.result,
        QueryExecution::Cancelled(settled) => {
            return Ok(cancellation_result(settled, "the call was cancelled"));
        }
        QueryExecution::TimedOut(settled) => {
            return Ok(cancellation_result(
                settled,
                "the database operation exceeded its 30-second cancellation-request deadline",
            ));
        }
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
                &request_context.id,
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

fn cancellation_result(settled: Option<SettledQuery>, message: &str) -> CallToolResponse {
    match settled.and_then(|settled| settled.cancel_request) {
        Some(Err(error)) => tools::error_result(error.to_string()),
        _ => tools::error_result(message),
    }
}

fn stopped_before_execute(
    source_cancelled: bool,
    deadline: tokio::time::Instant,
    cancellation: &CancellationToken,
) -> Option<CallToolResponse> {
    if source_cancelled {
        cancellation.cancel();
        Some(tools::error_result("the call was cancelled"))
    } else if tokio::time::Instant::now() >= deadline {
        cancellation.cancel();
        Some(tools::error_result(
            "the database operation exceeded its 30-second cancellation-request deadline",
        ))
    } else {
        None
    }
}

/// The query answer: columns, rows, counts, and evidence. A result over the
/// response bound is narrowed by dropping whole rows — measured on the FINAL
/// reply line, the JSON-RPC envelope with the ACTUAL request id around the
/// built result included, the narrowing note included, never the payload
/// alone (A922-5, D5; D9) — never sent fat. The narrowed answer stays
/// internally consistent: `row_count` is the row count sent, the note names
/// the byte cut — even when the engine had already truncated the result, the
/// truncation flags alone cannot say it — and the evidence copy reports the
/// sent rows with `truncated: true`; the un-narrowed execution facts stay in
/// the audit. Only when even the rowless result exceeds the bound is the
/// answer refused.
fn bounded_payload(
    policy: &ServePolicy,
    name: &str,
    connector: &dyn DatabaseConnector,
    result: &QueryResult,
    max_rows: usize,
    started_unix_ms: i64,
    id: &RequestId,
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
    let mut byte_cut = false;
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
        if byte_cut {
            payload["note"] = serde_json::Value::String(format!(
                "the response exceeded the byte bound and was cut to {}/{} rows",
                rows.len(),
                total
            ));
        }
        let built = CallToolResult::structured(payload);
        if policy.response_allowed(tools::response_wire_bytes(&built, id)) || rows.is_empty() {
            return tools::bounded_built_result(policy, built, id);
        }
        rows.truncate(rows.len() / 2);
        truncated = true;
        byte_cut = true;
        evidence.returned_rows = rows.len();
        evidence.truncated = true;
    }
}
