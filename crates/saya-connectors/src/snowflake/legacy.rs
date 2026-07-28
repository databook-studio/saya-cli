use reqwest::{StatusCode, header};
use saya_types::{ConnectionError, QueryRequest, QueryResult};
use serde_json::{Value, json};
use tokio::time::timeout;

use super::{auth, client::SnowflakeConnector, context, errors, legacy_chunks, sso};

pub(crate) async fn login(connector: &SnowflakeConnector) -> Result<String, ConnectionError> {
    let password = match &connector.auth {
        auth::Auth::Userpass(item) => {
            if let Some(token) = item.token.lock().await.clone() {
                return Ok(token);
            }
            item.password.clone()
        }
        auth::Auth::ExternalBrowser(item) => return sso::login(connector, item).await,
        _ => return Err(errors::auth()),
    };
    let url = format!("{}/session/v1/login-request", connector.origin);
    let mut data = json!({"LOGIN_NAME": connector.user, "PASSWORD": password, "ACCOUNT_NAME": connector.account_identifier});
    data.as_object_mut()
        .unwrap()
        .extend(context::fields(&connector.context));
    let body = json!({"data": data});
    let params = context::params(&connector.context);
    let response = timeout(
        connector.timeout,
        connector.client.post(url).query(&params).json(&body).send(),
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
    if let auth::Auth::Userpass(item) = &connector.auth {
        *item.token.lock().await = Some(token.clone());
    }
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
        return legacy_chunks::collect(connector, value, request.max_rows, request.sql).await;
    }
    Err(errors::auth())
}

async fn clear(connector: &SnowflakeConnector) {
    match &connector.auth {
        auth::Auth::Userpass(item) => *item.token.lock().await = None,
        auth::Auth::ExternalBrowser(item) => *item.token.lock().await = None,
        _ => {}
    }
}
fn expired(value: &Value) -> bool {
    value.pointer("/data/code").and_then(Value::as_str) == Some("390104")
        || value
            .pointer("/data/message")
            .and_then(Value::as_str)
            .is_some_and(|v| v.to_ascii_uppercase().contains("SESSION EXPIRED"))
}
