use super::api::ChildRunError;
use saya_agent::{CancellationReason, CancellationToken};
use std::{future::Future, time::Duration};

/// This is a join grace period, not proof of process termination. On expiry,
/// the abandoned loop's durable effect reservations remain held for reconciliation.
pub const CHILD_CANCEL_JOIN_GRACE: Duration = Duration::from_secs(3);

pub(super) async fn join_with_cancellation<T>(
    future: impl Future<Output = Result<T, ChildRunError>>,
    root: CancellationToken,
    child: CancellationToken,
) -> Result<T, ChildRunError> {
    tokio::pin!(future);
    tokio::select! {
        biased;
        _ = root.cancelled() => { child.cancel_with_reason(root.reason().unwrap_or(CancellationReason::OwnerFailure)); },
        _ = child.cancelled() => {},
        result = &mut future => return result,
    }
    match tokio::time::timeout(CHILD_CANCEL_JOIN_GRACE, &mut future).await {
        Ok(result) => result,
        Err(_) => Err(ChildRunError::Unsettled),
    }
}
