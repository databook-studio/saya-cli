use saya_types::ConnectionError;
use tokio::time::timeout;

use super::{PostgresConnector, errors};

pub(crate) async fn cancel(connector: &PostgresConnector) -> Result<(), ConnectionError> {
    let Some(pid) = *connector.active_pid.lock().await else {
        return Ok(());
    };
    timeout(
        connector.query_timeout,
        sqlx::query("SELECT pg_cancel_backend($1)")
            .bind(pid)
            .execute(&connector.pool),
    )
    .await
    .map_err(|_| ConnectionError::QueryFailed("PostgreSQL cancellation timed out".into()))?
    .map_err(errors::query)?;
    Ok(())
}
