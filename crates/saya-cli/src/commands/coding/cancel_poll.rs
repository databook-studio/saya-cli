//! The durable cancellation poll one running job watches.
//!
//! The poll is a monitor, not a cancellation order: a store that is briefly
//! unreachable (a lock wait on the store's one connection, a checkpoint in
//! flight) must not stop a healthy run. It is retried for a bounded window; a
//! store that stays unreachable that long is a failed owner, and the run stops.

use saya_agent::CancellationReason;
use saya_store::{SqliteStateStore, StoreError};
use saya_types::JobId;
use std::time::Duration;

/// How often the job's durable cancellation intent is read.
const POLL: Duration = Duration::from_millis(100);
/// How long the poll may keep failing to reach the store before the run is
/// stopped as an owner failure.
const UNAVAILABLE_CEILING: Duration = Duration::from_secs(15);

/// Watches the job's durable cancellation request until one arrives, or until
/// the store has been unreachable for the whole ceiling.
pub(super) async fn watch(store: &SqliteStateStore, job: &JobId) -> CancellationReason {
    let mut unavailable_since = None;
    loop {
        tokio::time::sleep(POLL).await;
        match store.coding_cancel_requested(job).await {
            Ok(false) => unavailable_since = None,
            Ok(true) => return CancellationReason::User,
            Err(StoreError::Unavailable) => {
                let since = *unavailable_since.get_or_insert_with(tokio::time::Instant::now);
                if since.elapsed() >= UNAVAILABLE_CEILING {
                    return CancellationReason::OwnerFailure;
                }
            }
            Err(_) => return CancellationReason::OwnerFailure,
        }
    }
}
