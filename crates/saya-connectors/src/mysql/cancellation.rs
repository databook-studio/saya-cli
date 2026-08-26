use saya_types::ConnectionError;
use tokio::time::timeout;

use super::{MySqlConnector, errors};

pub(crate) async fn cancel(connector: &MySqlConnector) -> Result<(), ConnectionError> {
    let Some(id) = *connector.active_id.lock().await else {
        // Nothing in flight: cancelling is a no-op, not an error.
        return Ok(());
    };
    // The id comes from CONNECTION_ID() as an integer and is interpolated
    // only into this KILL statement; KILL does not accept bound parameters.
    timeout(
        connector.query_timeout,
        sqlx::query(&format!("KILL QUERY {id}")).execute(&connector.pool),
    )
    .await
    .map_err(|_| ConnectionError::query_failed("MySQL cancellation timed out"))?
    .map_err(errors::query)?;
    Ok(())
}
