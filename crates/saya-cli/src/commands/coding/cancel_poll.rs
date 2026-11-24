//! The durable cancellation poll one running job watches.
//!
//! The poll is a monitor, not a cancellation order: a store that is briefly
//! unreachable (a lock wait on the store's one connection, a checkpoint in
//! flight) must not stop a healthy run. It is retried for a bounded window; a
//! store that stays unreachable that long is a failed owner, and the run stops.
//!
//! It runs as its own task ([`Poll`]) rather than as one arm of the owner's
//! wait, so the read it takes is always driven to completion: a read left in
//! flight by an owner nobody is polling would hold the store's one connection
//! for as long as the owner sits parked.

use saya_agent::CancellationReason;
use saya_store::{SqliteStateStore, StoreError};
use saya_types::JobId;
use std::time::Duration;

/// How often the job's durable cancellation intent is read.
const POLL: Duration = Duration::from_millis(100);
/// How long the poll may keep failing to reach the store before the run is
/// stopped as an owner failure.
const UNAVAILABLE_CEILING: Duration = Duration::from_secs(15);

/// The watch as its own task.
///
/// The owner it serves may be parked inside any of its waits — a provider call,
/// a child join — and a poll left inside that task would keep whatever store
/// read it had in flight for as long as the owner sat there. Owned by the
/// runtime instead, the read always finishes and gives the store's one
/// connection back, and the wait's end ends the task.
pub(super) struct Poll(tokio::task::JoinHandle<CancellationReason>);

impl Poll {
    pub(super) fn start(store: SqliteStateStore, job: JobId) -> Self {
        Self(tokio::spawn(async move { watch(&store, &job).await }))
    }

    /// The reason the poll stopped the run with. A poll task that cannot report
    /// one — it was aborted, or it panicked — is an owner failure like any other.
    pub(super) async fn reason(&mut self) -> CancellationReason {
        (&mut self.0)
            .await
            .unwrap_or(CancellationReason::OwnerFailure)
    }
}

impl Drop for Poll {
    fn drop(&mut self) {
        self.0.abort();
    }
}

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
