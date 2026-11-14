use std::future::Future;

use saya_agent::CancellationToken;
use saya_connectors::DatabaseConnector;
use saya_types::QueryRequest;

use crate::interactive::sql_operation::cancellation::{
    SettledQuery, execute_with_cancellation_outcome,
};

pub(super) enum QueryExecution {
    Completed(SettledQuery),
    Cancelled(Option<SettledQuery>),
    TimedOut(Option<SettledQuery>),
}

pub(super) async fn settle_query_execution<F, C>(
    connector: &dyn DatabaseConnector,
    request: QueryRequest,
    cancellation: &CancellationToken,
    source_cancelled: F,
    source_is_cancelled: C,
    deadline: tokio::time::Instant,
) -> QueryExecution
where
    F: Future<Output = ()>,
    C: Fn() -> bool,
{
    if source_is_cancelled() {
        cancellation.cancel();
        return QueryExecution::Cancelled(None);
    }
    if tokio::time::Instant::now() >= deadline {
        cancellation.cancel();
        return QueryExecution::TimedOut(None);
    }

    let execution = execute_with_cancellation_outcome(connector, request, cancellation);
    tokio::pin!(execution);
    tokio::select! {
        biased;
        _ = source_cancelled => {
            cancellation.cancel();
            QueryExecution::Cancelled(Some(execution.await))
        }
        _ = tokio::time::sleep_until(deadline) => {
            cancellation.cancel();
            QueryExecution::TimedOut(Some(execution.await))
        }
        settled = &mut execution => QueryExecution::Completed(settled),
    }
}
