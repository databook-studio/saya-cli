use saya_types::ConnectionError;
use tokio::time::timeout;

use super::{PostgresConnector, errors};
use crate::CancelRequestOutcome;

pub(crate) async fn cancel(connector: &PostgresConnector) -> Result<(), ConnectionError> {
    request_cancel(connector).await.map(|_| ())
}

pub(crate) async fn request_cancel(
    connector: &PostgresConnector,
) -> Result<CancelRequestOutcome, ConnectionError> {
    let Some(pid) = *connector.active_pid.lock().await else {
        return Ok(CancelRequestOutcome::NoActiveOperation);
    };
    let accepted = timeout(
        connector.query_timeout,
        sqlx::query_scalar::<_, bool>("SELECT pg_cancel_backend($1)")
            .bind(pid)
            .fetch_one(&connector.pool),
    )
    .await
    .map_err(|_| ConnectionError::query_failed("PostgreSQL cancellation timed out"))?
    .map_err(errors::query)?;
    if accepted {
        Ok(CancelRequestOutcome::RemoteRequestAccepted)
    } else {
        Err(ConnectionError::query_failed(
            "PostgreSQL did not accept cancellation request",
        ))
    }
}
