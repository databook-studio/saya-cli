use std::time::Duration;

use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::{net::TcpListener, time::timeout};
use url::Url;

use saya_types::ConnectionError;

use super::{auth::ExternalBrowser, client::SnowflakeConnector, context, errors, sso_callback};

pub(crate) async fn login(
    connector: &SnowflakeConnector,
    auth: &ExternalBrowser,
) -> Result<String, ConnectionError> {
    if !auth.enabled {
        return Err(errors::interactive());
    }
    if let Some(token) = auth.token.lock().await.clone() {
        return Ok(token);
    }
    timeout(connector.sso_timeout, login_flow(connector, auth))
        .await
        .map_err(|_| errors::auth())?
}

async fn login_flow(
    connector: &SnowflakeConnector,
    auth: &ExternalBrowser,
) -> Result<String, ConnectionError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| errors::auth())?;
    let port = listener.local_addr().map_err(|_| errors::auth())?.port();
    let request = json!({"data": {
        "ACCOUNT_NAME": connector.account_identifier,
        "LOGIN_NAME": connector.user,
        "AUTHENTICATOR": "externalbrowser",
        "BROWSER_MODE_REDIRECT_PORT": port,
        "CLIENT_APP_ID": "saya-cli",
        "CLIENT_APP_VERSION": "0.1"
    }});
    let response = connector
        .client
        .post(format!(
            "{}/session/authenticator-request",
            connector.origin
        ))
        .json(&request)
        .send()
        .await
        .map_err(|_| errors::connect())?;
    let value: Value = checked_json(response).await.map_err(|_| errors::auth())?;
    if value.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(errors::auth());
    }
    let data = value.get("data").ok_or_else(errors::auth)?;
    let url = data
        .get("ssoUrl")
        .and_then(Value::as_str)
        .ok_or_else(errors::auth)?;
    validate_url(url)?;
    let proof_key = data.get("proofKey").and_then(Value::as_str);
    (connector.browser_opener)(url).map_err(|_| errors::auth())?;
    let token = sso_callback::capture_token(&listener, connector.sso_timeout).await?;
    let mut data = json!({
        "ACCOUNT_NAME": connector.account_identifier,
        "LOGIN_NAME": connector.user,
        "PASSWORD": null,
        "TOKEN": token,
        "PROOF_KEY": proof_key,
        "AUTHENTICATOR": "externalbrowser",
        "CLIENT_APP_ID": "saya-cli",
        "CLIENT_APP_VERSION": "0.1"
    });
    data.as_object_mut()
        .unwrap()
        .extend(context::fields(&connector.context));
    let body = json!({"data": data});
    let params = context::params(&connector.context);
    let response = connector
        .client
        .post(format!("{}/session/v1/login-request", connector.origin))
        .query(&params)
        .json(&body)
        .send()
        .await
        .map_err(|_| errors::connect())?;
    let value = checked_json(response).await.map_err(|_| errors::auth())?;
    if value.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(errors::auth());
    }
    let session = value
        .pointer("/data/token")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(errors::auth)?
        .to_owned();
    *auth.token.lock().await = Some(session.clone());
    Ok(session)
}

async fn checked_json(response: reqwest::Response) -> Result<Value, ()> {
    if !response.status().is_success() || response.status() == StatusCode::NO_CONTENT {
        return Err(());
    }
    response.json().await.map_err(|_| ())
}

fn validate_url(value: &str) -> Result<(), ConnectionError> {
    let parsed = Url::parse(value).map_err(|_| errors::auth())?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(errors::auth());
    }
    Ok(())
}

pub(crate) fn auth_timeout() -> Duration {
    Duration::from_secs(120)
}
