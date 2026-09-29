//! The `query` tool (task Db): one bounded, read-only SQL statement through
//! the connector's safety gate — the same `execute` path `saya query` takes,
//! with the same configured row cap — returned with execution evidence
//! naming this server as the source. A cancelled call stops waiting; where
//! the connector supports cancellation, it is attempted before the unsent
//! error is dropped.

use std::time::Instant;

use rmcp::{
    RoleServer,
    model::{CallToolRequestParams, CallToolResponse},
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
            let payload = payload(
                policy,
                name,
                connector.as_ref(),
                &result,
                max_rows,
                started_unix_ms,
            );
            tools::bounded_result(policy, payload)
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

/// The query answer: columns, rows, counts, and evidence. A payload over the
/// response bound is narrowed by dropping whole rows (truncated:true and a
/// note), never sent fat.
fn payload(
    policy: &ServePolicy,
    name: &str,
    connector: &dyn DatabaseConnector,
    result: &QueryResult,
    max_rows: usize,
    started_unix_ms: i64,
) -> serde_json::Value {
    let evidence = ExecutionEvidence::for_result(
        result,
        ExecutionEvidenceArgs {
            execution_id: ExecutionEvidence::new_execution_id(started_unix_ms, 0),
            connection_label: name.to_owned(),
            connection_identity: None,
            dialect: connector.dialect(),
            max_rows,
            started_unix_ms,
            finished_unix_ms: unix_now_ms(),
            source: EvidenceSource::DirectSql,
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
            "source": "mcp",
            "evidence": evidence,
        });
        let size = serde_json::to_vec(&payload).map_or(usize::MAX, |bytes| bytes.len());
        if policy.response_allowed(size) || rows.is_empty() {
            if !policy.response_allowed(size) {
                payload["note"] = serde_json::Value::String(
                    "the response exceeded the byte bound; no row could be kept".into(),
                );
            }
            return payload;
        }
        rows.truncate(rows.len() / 2);
        truncated = true;
        payload["note"] = serde_json::Value::String(format!(
            "the response exceeded the byte bound and was cut to {}/{} rows",
            rows.len(),
            total
        ));
    }
}
