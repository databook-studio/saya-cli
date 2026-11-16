use saya_store::{CodingRecoveryStatus, SqliteStateStore, StoreError};
use saya_types::JobId;

/// A transactional durable inspection. Unknown command/process identity is
/// represented by unresolved effect ownership, never by trusting a reused PID.
/// Use the original effect owner's reconciliation path before resume; this
/// operation does not kill a process, refund a charge, or execute a child.
pub async fn inspect_recovery(
    store: &SqliteStateStore,
    job: &JobId,
) -> Result<CodingRecoveryStatus, StoreError> {
    store.coding_recovery_status(job).await
}
