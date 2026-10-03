use saya_agent::CancellationToken;
use saya_connectors::DatabaseConnector;
use saya_types::{ConnectionError, QueryRequest, QueryResult};

/// Requests cancellation while retaining and polling the real execute future
/// until the connector reports its terminal result.
pub(super) async fn execute_with_cancellation(
    connector: &dyn DatabaseConnector,
    request: QueryRequest,
    cancellation: &CancellationToken,
) -> Result<QueryResult, ConnectionError> {
    let execution = connector.execute(request);
    tokio::pin!(execution);
    tokio::select! {
        biased;
        result = &mut execution => result,
        _ = cancellation.cancelled() => {
            let request = connector.request_cancel();
            tokio::pin!(request);
            let (result, _outcome) = tokio::join!(&mut execution, &mut request);
            result
        }
    }
}
