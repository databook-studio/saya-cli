pub(crate) async fn query_with_context(
    operation_entry: &OperationEntry<'_>,
    sql: &str,
    max_rows: usize,
    store: Option<&SqliteStateStore>,
    profile_id: Option<&str>,
    context: &ToolExecutionContext,
    timeout: Option<std::time::Duration>,
) -> Result<serde_json::Value, ToolError> {
    query_typed(
        operation_entry,
        sql,
        max_rows,
        store,
        profile_id,
        context,
        timeout,
    )
    .await
    .map(|executed| executed.value)
}

/// Executes a registry-owned query for a tool that deliberately has its own
/// result policy (the chart tool). It shares only operation ownership and
/// cancellation with the ordinary query path, not auditing or model shaping.
pub(crate) async fn operation_execute(
    operation_entry: &OperationEntry<'_>,
    request: QueryRequest,
    context: &ToolExecutionContext,
) -> Result<QueryResult, ToolError> {
    match operation::execute(
        operation_entry,
        request,
        &context.cancellation,
        None,
        operation_entry.metrics,
    )
    .await
    {
        Ok(result) if !context.cancellation.is_cancelled() => Ok(result),
        Ok(_) | Err(operation::OperationError::Cancelled | operation::OperationError::TimedOut) => {
            Err(cancelled_tool_error())
        }
        Err(operation::OperationError::Connector(error)) => {
            Err(ToolError::QueryFailedDetail(error.to_string()))
        }
    }
}

/// One executed agent query: the serialized value the model receives, the
/// typed result it was serialized from, the row cap actually applied, and the
/// execution window in unix milliseconds.
pub(crate) struct ExecutedQuery {
    pub(crate) value: serde_json::Value,
    pub(crate) result: QueryResult,
    pub(crate) row_cap: usize,
    pub(crate) started_unix_ms: i64,
    pub(crate) finished_unix_ms: i64,
}

/// The typed variant of [`query`]: the same path, audit, and value — plus the
/// typed result, the applied row cap, and the execution window, so the
/// capture hook can hand the TUI exactly what the model saw.
pub(crate) async fn query_typed(
    operation_entry: &OperationEntry<'_>,
    sql: &str,
    max_rows: usize,
    store: Option<&SqliteStateStore>,
    profile_id: Option<&str>,
    context: &ToolExecutionContext,
    timeout: Option<std::time::Duration>,
) -> Result<ExecutedQuery, ToolError> {
    let started = Instant::now();
    let started_unix_ms = unix_now_ms();
    // Cap the rows the MODEL sees (not the /sql display path, which keeps max_rows).
    let model_rows = model_row_cap(max_rows);
    match operation::execute(
        operation_entry,
        QueryRequest::new(sql, model_rows),
        &context.cancellation,
        timeout,
        operation_entry.metrics,
    )
    .await
    {
        Ok(result) => {
            if context.cancellation.is_cancelled() {
                return Err(cancelled_tool_error());
            }
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
            let value =
                serde_json::to_value(&result).map_err(|_| ToolError::QueryResultUnavailable)?;
            let finished_unix_ms = unix_now_ms();
            Ok(ExecutedQuery {
                value,
                result,
                row_cap: model_rows,
                started_unix_ms,
                finished_unix_ms,
            })
        }
        Err(operation::OperationError::Connector(error)) => {
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
        Err(operation::OperationError::Cancelled) => Err(cancelled_tool_error()),
        Err(operation::OperationError::TimedOut) => Err(ToolError::QueryTimedOut),
    }
}

fn cancelled_tool_error() -> ToolError {
    ToolError::QueryFailedDetail("query cancelled".into())
}

/// Current wall clock in unix milliseconds (0 before the epoch).
fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
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
