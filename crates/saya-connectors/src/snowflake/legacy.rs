use reqwest::{StatusCode, header};
use saya_types::{ConnectionError, QueryRequest, QueryResult};
use serde_json::{Value, json};
use tokio::time::timeout;

use super::{auth, client::SnowflakeConnector, errors, result};

pub(crate) async fn login(connector: &SnowflakeConnector) -> Result<String, ConnectionError> {
    let userpass = match &connector.auth {
        auth::Auth::Userpass(item) => item,
        _ => return Err(errors::auth()),
    };
    if let Some(token) = userpass.token.lock().await.clone() {
        return Ok(token);
    }
    let url = format!("{}/session/v1/login-request", connector.origin);
    let body = json!({"data": {"LOGIN_NAME": connector.user, "PASSWORD": userpass.password, "ACCOUNT_NAME": connector.account, "WAREHOUSE": connector.context.warehouse, "DATABASE_NAME": connector.context.database, "SCHEMA_NAME": connector.context.schema, "ROLE_NAME": connector.context.role}});
    let response = timeout(
        connector.timeout,
        connector.client.post(url).json(&body).send(),
    )
    .await
    .map_err(|_| errors::connect())?
    .map_err(|_| errors::connect())?;
    if !response.status().is_success() {
        return Err(errors::auth());
    }
    let value: Value = response.json().await.map_err(|_| errors::auth())?;
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(errors::auth());
    }
    let token = value
        .get("data")
        .and_then(|d| d.get("token"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(errors::auth)?
        .to_owned();
    *userpass.token.lock().await = Some(token.clone());
    Ok(token)
}

pub(crate) async fn execute(
    connector: &SnowflakeConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let sql = crate::prepare_snowflake_sql(&request.sql, request.max_rows)?;
    for attempt in 0..2 {
        let token = login(connector).await?;
        let url = format!("{}/queries/v1/query-request", connector.origin);
        let body = json!({"sqlText": sql, "async": false, "sequenceId": 1});
        let response = timeout(
            connector.timeout,
            connector
                .client
                .post(url)
                .header(
                    header::AUTHORIZATION,
                    format!("Snowflake Token=\"{token}\""),
                )
                .json(&body)
                .send(),
        )
        .await
        .map_err(|_| errors::query())?
        .map_err(|_| errors::connect())?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED && attempt == 0 {
            clear(connector).await;
            continue;
        }
        let value: Value = response.json().await.map_err(|_| errors::query())?;
        if expired(&value) && attempt == 0 {
            clear(connector).await;
            continue;
        }
        if !status.is_success() || value.get("success").and_then(Value::as_bool) == Some(false) {
            return Err(errors::query());
        }
        return with_chunks(connector, value, request.max_rows, request.sql).await;
    }
    Err(errors::auth())
}

async fn clear(connector: &SnowflakeConnector) {
    if let auth::Auth::Userpass(item) = &connector.auth {
        *item.token.lock().await = None;
    }
}
fn expired(value: &Value) -> bool {
    value.pointer("/data/code").and_then(Value::as_str) == Some("390104")
        || value
            .pointer("/data/message")
            .and_then(Value::as_str)
            .is_some_and(|v| v.to_ascii_uppercase().contains("SESSION EXPIRED"))
}

async fn with_chunks(
    connector: &SnowflakeConnector,
    value: Value,
    max: usize,
    original: String,
) -> Result<QueryResult, ConnectionError> {
    let mut output = result::result(&value, max.saturating_add(1), original.clone())?;
    let data = value.get("data").unwrap_or(&value);
    let headers = data.get("chunkHeaders").and_then(Value::as_object);
    let chunks = data
        .get("chunks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Sequential fetching is the bounded ordered strategy: one in-flight chunk preserves declaration order.
    for chunk in chunks {
        if output.rows.len() > max {
            break;
        }
        let url = chunk
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(errors::query)?;
        let mut request = connector.client.get(url);
        for name in [
            "x-amz-server-side-encryption-customer-key",
            "x-amz-server-side-encryption-customer-key-md5",
        ] {
            if let Some(value) = headers
                .and_then(|item| item.get(name))
                .and_then(Value::as_str)
            {
                request = request.header(name, value);
            }
        }
        let response = timeout(connector.timeout, request.send())
            .await
            .map_err(|_| errors::query())?
            .map_err(|_| errors::query())?;
        if !response.status().is_success() {
            return Err(errors::query());
        }
        let text = response.text().await.map_err(|_| errors::query())?;
        output.rows.extend(result::chunk_rows(&text)?);
    }
    result::bounded(output.columns, output.rows, max, original)
}
