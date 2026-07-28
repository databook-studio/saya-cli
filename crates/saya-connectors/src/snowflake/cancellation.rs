use reqwest::header;
use saya_types::ConnectionError;
use tokio::time::timeout;

use super::{auth, client::SnowflakeConnector, errors};

pub(crate) async fn cancel(connector: &SnowflakeConnector) -> Result<(), ConnectionError> {
    let handle = connector
        .active
        .lock()
        .await
        .clone()
        .filter(|item| uuid::Uuid::parse_str(item).is_ok())
        .ok_or(ConnectionError::Unsupported(
            "no active Snowflake statement".into(),
        ))?;
    let auth::Auth::Keypair(key) = &connector.auth else {
        return Err(ConnectionError::Unsupported(
            "Snowflake cancellation is unavailable for this auth flow".into(),
        ));
    };
    let token = auth::jwt(&connector.account, &connector.user, key).map_err(|_| errors::auth())?;
    let url = format!("{}/api/v2/statements/{handle}/cancel", connector.origin);
    let response = timeout(
        connector.timeout,
        connector
            .client
            .post(url)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header("X-Snowflake-Authorization-Token-Type", "KEYPAIR_JWT")
            .send(),
    )
    .await
    .map_err(|_| errors::query())?
    .map_err(|_| errors::connect())?;
    response
        .status()
        .is_success()
        .then_some(())
        .ok_or_else(errors::query)
}
