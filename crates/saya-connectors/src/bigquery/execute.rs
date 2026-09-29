use saya_types::{BoundParam, ConnectionError, QueryRequest, QueryResult, SqlDialect};
use serde_json::Value;

use super::BigQueryConnector;
use super::diagnose;
use super::errors;
use super::request::{dry_run_body, parse_result, query_body, query_parameters};

/// Runs a read-only query: the safety layer narrows the SQL, a dry-run refuses
/// an over-budget estimate before execution, then the synchronous `jobs.query`
/// returns a bounded result with `maximumBytesBilled` as a second bound.
pub(crate) async fn query(
    connector: &BigQueryConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let (sql, parameters) = prepare(&request.sql, request.max_rows, &request.params)?;
    let original_sql = request.sql;
    let max_rows = request.max_rows;
    let token = connector.token().await?;
    refuse_if_over_budget(connector, &sql, &token, parameters.as_deref()).await?;
    let response = connector
        .post(
            &connector.query_url(),
            &token,
            query_body(
                &sql,
                max_rows,
                connector.max_bytes_billed,
                connector.location.as_deref(),
                parameters.as_deref(),
            ),
        )
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = crate::common::read_text(response, crate::common::MAX_HTTP_ERROR_BYTES)
            .await
            .unwrap_or_default();
        return Err(diagnose::query_failure(status, &body));
    }
    let value: Value = crate::common::read_json(response, crate::common::MAX_HTTP_BODY_BYTES)
        .await
        .map_err(|_| errors::body_decode())?;
    Ok(parse_result(value, max_rows, original_sql))
}

/// Validates auth by running a trivial read. Reuses the full path so a closed
/// port, a bad key, or an unreadable response all surface as a connection or
/// authentication failure.
pub(crate) async fn ping(connector: &BigQueryConnector) -> Result<(), ConnectionError> {
    query(connector, QueryRequest::new("SELECT 1", 1)).await?;
    Ok(())
}

/// Submits a dry-run and refuses the query when the estimated bytes scanned
/// exceed the configured budget, before the query is allowed to execute. The
/// estimate is a heuristic refusal; `maximumBytesBilled` on the real job is
/// the hard backstop that catches a query whose actual cost diverges. The
/// dry-run carries the same positional parameters as the query itself, so the
/// estimate is computed against the statement that will actually run.
async fn refuse_if_over_budget(
    connector: &BigQueryConnector,
    sql: &str,
    token: &str,
    parameters: Option<&[Value]>,
) -> Result<(), ConnectionError> {
    let response = connector
        .post(
            &connector.jobs_url(),
            token,
            dry_run_body(
                sql,
                connector.max_bytes_billed,
                connector.location.as_deref(),
                parameters,
            ),
        )
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = crate::common::read_text(response, crate::common::MAX_HTTP_ERROR_BYTES)
            .await
            .unwrap_or_default();
        return Err(diagnose::query_failure(status, &body));
    }
    let value: Value = crate::common::read_json(response, crate::common::MAX_HTTP_BODY_BYTES)
        .await
        .map_err(|_| errors::body_decode())?;
    let estimate = value
        .get("statistics")
        .and_then(|s| s.get("query"))
        .and_then(|q| q.get("totalBytesProcessed"))
        .and_then(Value::as_str)
        .and_then(|n| n.parse::<u64>().ok())
        .unwrap_or(0);
    if estimate > connector.max_bytes_billed {
        return Err(ConnectionError::query_failed(
            "BigQuery query rejected: estimated bytes scanned exceed the configured byte budget",
        ));
    }
    Ok(())
}

/// Prepares the statement for execution. Parameter-free SQL keeps today's
/// prepare path byte-for-byte; parameterized SQL is rewritten to `?` markers
/// with the positional `queryParameters` — values never touch the text.
fn prepare(
    sql: &str,
    max_rows: usize,
    params: &[BoundParam],
) -> Result<(String, Option<Vec<Value>>), ConnectionError> {
    if params.is_empty() {
        return Ok((crate::prepare_bigquery_sql(sql, max_rows)?, None));
    }
    let prepared = crate::prepare_with_params(sql, max_rows, SqlDialect::BigQuery, params)?;
    let parsed = crate::binds::parse_bind_values(&prepared.values)?;
    Ok((prepared.sql, Some(query_parameters(&parsed)?)))
}
