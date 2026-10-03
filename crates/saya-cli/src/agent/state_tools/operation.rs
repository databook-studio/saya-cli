use std::future::{Future, pending};
use std::time::Duration;

use futures_util::poll;
use saya_agent::CancellationToken;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree};
use tokio::sync::MutexGuard;

use crate::connection::OperationEntry;

/// An operation settled after the caller cancelled or its local deadline won.
pub(super) enum OperationError {
    Cancelled,
    TimedOut,
    Connector(ConnectionError),
}

pub(super) async fn execute(
    operation: &OperationEntry<'_>,
    request: QueryRequest,
    cancellation: &CancellationToken,
    timeout: Option<Duration>,
) -> Result<QueryResult, OperationError> {
    let _lease = acquire(operation, cancellation).await?;
    settle(
        operation.entry.connector.as_ref(),
        operation.entry.connector.execute(request),
        cancellation,
        timeout,
    )
    .await
}

pub(super) async fn schema(
    operation: &OperationEntry<'_>,
    cancellation: &CancellationToken,
) -> Result<SchemaTree, OperationError> {
    let _lease = acquire(operation, cancellation).await?;
    settle(
        operation.entry.connector.as_ref(),
        operation.entry.connector.schema(),
        cancellation,
        None,
    )
    .await
}

async fn acquire<'a>(
    operation: &'a OperationEntry<'_>,
    cancellation: &CancellationToken,
) -> Result<MutexGuard<'a, ()>, OperationError> {
    if cancellation.is_cancelled() {
        return Err(OperationError::Cancelled);
    }
    let lease = tokio::select! {
        lease = operation.lease.lock() => lease,
        _ = cancellation.cancelled() => return Err(OperationError::Cancelled),
    };
    if cancellation.is_cancelled() {
        return Err(OperationError::Cancelled);
    }
    Ok(lease)
}

async fn settle<T, F>(
    connector: &dyn saya_connectors::DatabaseConnector,
    execution: F,
    cancellation: &CancellationToken,
    timeout: Option<Duration>,
) -> Result<T, OperationError>
where
    F: Future<Output = Result<T, ConnectionError>>,
{
    tokio::pin!(execution);
    if let std::task::Poll::Ready(result) = poll!(&mut execution) {
        return result.map_err(OperationError::Connector);
    }
    if cancellation.is_cancelled() {
        return cancel_and_settle(connector, execution, OperationError::Cancelled).await;
    }
    let deadline = async {
        match timeout {
            Some(timeout) => tokio::time::sleep(timeout).await,
            None => pending::<()>().await,
        }
    };
    tokio::pin!(deadline);
    tokio::select! {
        result = &mut execution => result.map_err(OperationError::Connector),
        _ = cancellation.cancelled() => {
            cancel_and_settle(connector, execution, OperationError::Cancelled).await
        }
        _ = &mut deadline => cancel_and_settle(connector, execution, OperationError::TimedOut).await,
    }
}

async fn cancel_and_settle<T, F>(
    connector: &dyn saya_connectors::DatabaseConnector,
    execution: std::pin::Pin<&mut F>,
    outcome: OperationError,
) -> Result<T, OperationError>
where
    F: Future<Output = Result<T, ConnectionError>>,
{
    // A connector outcome describes only the cancellation request. Retain and
    // poll native execution concurrently until it settles before releasing the
    // entry lease; neither outcome is claimed as a server-stop acknowledgement.
    let (_request, _settled) = tokio::join!(connector.request_cancel(), execution);
    Err(outcome)
}
