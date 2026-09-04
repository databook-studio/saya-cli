use saya_types::{ConnectionError, QueryRequest, QueryResult};
use serde_json::Value;

use super::BigQueryConnector;
use super::errors;
use super::request::{dry_run_body, parse_result, query_body};

/// Runs a read-only query: the safety layer narrows the SQL, a dry-run refuses
/// an over-budget estimate before execution, then the synchronous `jobs.query`
/// returns a bounded result with `maximumBytesBilled` as a second bound.
pub(crate) async fn query(
    connector: &BigQueryConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let sql = crate::prepare_bigquery_sql(&request.sql, request.max_rows)?;
    let token = connector.token().await?;
    refuse_if_over_budget(connector, &sql, &token).await?;
    let response = connector
        .post(
            &connector.query_url(),
            &token,
            query_body(
                &sql,
                request.max_rows,
                connector.max_bytes_billed,
                connector.location.as_deref(),
            ),
        )
        .await?;
    if !response.status().is_success() {
        return Err(errors::query_status(response.status()));
    }
    let value: Value = response.json().await.map_err(errors::body)?;
    Ok(parse_result(value, request.max_rows, request.sql))
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
/// the hard backstop that catches a query whose actual cost diverges.
async fn refuse_if_over_budget(
    connector: &BigQueryConnector,
    sql: &str,
    token: &str,
) -> Result<(), ConnectionError> {
    let response = connector
        .post(
            &connector.jobs_url(),
            token,
            dry_run_body(
                sql,
                connector.max_bytes_billed,
                connector.location.as_deref(),
            ),
        )
        .await?;
    if !response.status().is_success() {
        return Err(errors::query_status(response.status()));
    }
    let value: Value = response.json().await.map_err(errors::body)?;
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
