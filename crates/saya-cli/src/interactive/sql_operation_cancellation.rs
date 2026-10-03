use std::sync::atomic::{AtomicBool, Ordering};

use saya_agent::CancellationToken;
use saya_connectors::{CancelRequestOutcome, DatabaseConnector};
use saya_types::{ConnectionError, QueryRequest, QueryResult};

pub(crate) struct SettledQuery {
    pub(crate) result: Result<QueryResult, ConnectionError>,
    pub(crate) cancel_request: Option<Result<CancelRequestOutcome, ConnectionError>>,
}

/// Requests cancellation while retaining and polling the real execute future
/// until the connector reports its terminal result.
pub(crate) async fn execute_with_cancellation(
    connector: &dyn DatabaseConnector,
    request: QueryRequest,
    cancellation: &CancellationToken,
) -> Result<QueryResult, ConnectionError> {
    execute_with_cancellation_outcome(connector, request, cancellation)
        .await
        .result
}

pub(crate) async fn execute_with_cancellation_outcome(
    connector: &dyn DatabaseConnector,
    request: QueryRequest,
    cancellation: &CancellationToken,
) -> SettledQuery {
    if cancellation.is_cancelled() {
        return SettledQuery {
            result: Err(ConnectionError::cancelled()),
            cancel_request: None,
        };
    }
    let started = AtomicBool::new(false);
    let execution = async {
        started.store(true, Ordering::Relaxed);
        connector.execute(request).await
    };
    tokio::pin!(execution);
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            if !started.load(Ordering::Relaxed) {
                return SettledQuery {
                    result: Err(ConnectionError::cancelled()),
                    cancel_request: None,
                };
            }
            let request = connector.request_cancel();
            tokio::pin!(request);
            let (result, cancel_request) = tokio::join!(&mut execution, &mut request);
            SettledQuery { result, cancel_request: Some(cancel_request) }
        }
        result = &mut execution => SettledQuery { result, cancel_request: None },
    }
}
