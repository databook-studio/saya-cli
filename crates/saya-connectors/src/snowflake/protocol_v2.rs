use std::time::Duration;

use reqwest::{StatusCode, header};
use saya_types::{BoundParam, ConnectionError, QueryRequest, QueryResult, SqlDialect};
use serde_json::{Value, json};
use tokio::time::{sleep, timeout};

use super::{auth, bindings, client::SnowflakeConnector, diagnose, errors, result, status_url};

pub(crate) async fn execute(
    connector: &SnowflakeConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let _in_flight = connector.in_flight.lock().await;
    let (sql, bound) = prepare(&request.sql, request.max_rows, &request.params)?;
    let original_sql = request.sql;
    let max_rows = request.max_rows;
    let token = auth::jwt(
        &connector.account,
        &connector.user,
        match &connector.auth {
            auth::Auth::Keypair(key) => key,
            _ => return Err(errors::auth()),
        },
    )
    .map_err(|_| errors::auth())?;
    let url = format!("{}/api/v2/statements", connector.origin);
    let mut body = json!({"statement": sql, "timeout": connector.timeout.as_secs(), "database": connector.context.database, "schema": connector.context.schema, "warehouse": connector.context.warehouse, "role": connector.context.role});
    // Snowflake replaces the `?` markers with these values server-side;
    // bindings are documented as unsupported for multi-statement requests,
    // and the safety layer only ever submits one statement.
    if let Some(bound) = bound {
        body["bindings"] = bound;
    }
    let response = send(
        connector.client.post(url),
        &token,
        Some(body),
        connector.timeout,
    )
    .await?;
    *connector.active.lock().await = status_url::handle(&response.1);
    let output = async {
        let value = poll(connector, &token, response).await?;
        *connector.active.lock().await = status_url::handle(&value);
        collect(connector, &token, value, max_rows, original_sql).await
    }
    .await;
    *connector.active.lock().await = None;
    output
}

/// Prepares the statement for execution. Parameter-free SQL keeps today's
/// prepare path byte-for-byte; parameterized SQL is rewritten to `?` markers
/// with the 1-based bindings object — values never touch the text.
fn prepare(
    sql: &str,
    max_rows: usize,
    params: &[BoundParam],
) -> Result<(String, Option<Value>), ConnectionError> {
    if params.is_empty() {
        return Ok((crate::prepare_snowflake_sql(sql, max_rows)?, None));
    }
    let prepared = crate::prepare_with_params(sql, max_rows, SqlDialect::Snowflake, params)?;
    let parsed = crate::binds::parse_bind_values(&prepared.values)?;
    Ok((prepared.sql, Some(bindings::bindings(&parsed)?)))
}

async fn send(
    request: reqwest::RequestBuilder,
    token: &str,
    body: Option<Value>,
    deadline: Duration,
) -> Result<(StatusCode, Value), ConnectionError> {
    let response = timeout(deadline, {
        let request = request
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header("X-Snowflake-Authorization-Token-Type", "KEYPAIR_JWT");
        match body {
            Some(body) => request.json(&body).send(),
            None => request.send(),
        }
    })
    .await
    .map_err(|_| errors::query())?
    .map_err(|_| errors::connect())?;
    let status = response.status();
    let value = crate::common::read_json(response, crate::common::MAX_HTTP_BODY_BYTES)
        .await
        .map_err(|_| errors::query())?;
    if status.is_client_error() && status != StatusCode::REQUEST_TIMEOUT {
        return Err(if status == StatusCode::UNAUTHORIZED {
            errors::auth()
        } else {
            diagnose::query_failure(&value)
        });
    }
    Ok((status, value))
}

async fn poll(
    connector: &SnowflakeConnector,
    token: &str,
    mut response: (StatusCode, Value),
) -> Result<Value, ConnectionError> {
    let started = tokio::time::Instant::now();
    let mut pause = Duration::from_millis(50);
    while response.0 == StatusCode::ACCEPTED || response.0 == StatusCode::REQUEST_TIMEOUT {
        if started.elapsed() >= connector.timeout {
            return Err(errors::query());
        }
        sleep(pause).await;
        pause = (pause * 2).min(Duration::from_millis(500));
        let location = response
            .1
            .get("statementStatusUrl")
            .and_then(Value::as_str)
            .ok_or_else(errors::query)?;
        let url = status_url::same_origin(&connector.origin, location).ok_or_else(errors::query)?;
        response = send(
            connector.client.get(url),
            token,
            None,
            connector.timeout.saturating_sub(started.elapsed()),
        )
        .await?;
    }
    if !response.0.is_success() {
        return Err(errors::query());
    }
    Ok(response.1)
}

async fn collect(
    connector: &SnowflakeConnector,
    token: &str,
    value: Value,
    max: usize,
    original: String,
) -> Result<QueryResult, ConnectionError> {
    let mut output = result::result(&value, max.saturating_add(1), original.clone())?;
    let partitions = value
        .get("resultSetMetaData")
        .or_else(|| value.get("data").and_then(|d| d.get("resultSetMetaData")))
        .and_then(|v| v.get("partitionInfo"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let handle = value
        .get("statementHandle")
        .or_else(|| value.get("data").and_then(|d| d.get("statementHandle")))
        .and_then(Value::as_str);
    for partition in 1..partitions {
        if output.rows.len() > max {
            break;
        }
        let handle = handle.ok_or_else(errors::query)?;
        let url = format!(
            "{}/api/v2/statements/{handle}?partition={partition}",
            connector.origin
        );
        let (_, extra) = send(connector.client.get(url), token, None, connector.timeout).await?;
        let extra = result::result(&extra, max.saturating_add(1), original.clone())?;
        output.rows.extend(extra.rows);
    }
    result::bounded(output.columns, output.rows, max, original)
}
