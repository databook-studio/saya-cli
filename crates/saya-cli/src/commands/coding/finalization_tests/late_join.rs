//! The late follow-up a run must join before it settles.
//!
//! The child was accepted before finalization started and its model request is
//! still held, so `finalize_run` cannot return: an accepted unfinished child
//! and a settled root cannot coexist. What the case asserts is that ordering —
//! finalization is still in flight while the child is unfinished, the writes it
//! makes next are the settled ones, and the child's history is complete — never
//! how long any step took.
//!
//! The store reads go through [`job_state`] because they share the store's one
//! connection with the run's own durable-cancel poll (`cancel_poll`), which
//! reads it every 100 ms while finalization waits. A read refused there is a
//! busy store, which the product itself treats as transient; retrying it is the
//! same reading, not a weaker one.

use super::super::super::{ChildFollowUpStarted, NativeCodingJob};
use crate::coding_test_support::{follow_up_fixture::Fixture, follow_up_provider};
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{JobId, JobState};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the store may keep answering `Unavailable` before a read is a real
/// failure: a load ceiling, not a product bound. The worker child these cases
/// run in is capped at ninety seconds, so one retried read stays well inside it.
const STORE_CEILING: Duration = Duration::from_secs(20);

/// How long one settled write may take after the held request is released: a
/// load ceiling too, the same thirty seconds [`super::wait_for`] allows the run
/// to reach its own milestones, and short enough to leave the child's ninety.
pub(super) const SETTLE_CEILING: Duration = Duration::from_secs(30);

/// The job's state, retried while the store reports itself busy.
pub(super) async fn job_state(store: &SqliteStateStore, job: &JobId) -> JobState {
    let start = Instant::now();
    loop {
        match store.get_coding_job(job).await {
            Ok(Some(record)) => return record.state,
            Ok(None) => panic!("the run's job row is in the store"),
            Err(StoreError::Unavailable) if start.elapsed() < STORE_CEILING => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(error) => panic!(
                "the run's job row reads after {} ms: {error}",
                start.elapsed().as_millis()
            ),
        }
    }
}

/// The late follow-up: finalization joins it before it settles the run.
pub(super) async fn joins_before_settling(
    resumed: &Arc<NativeCodingJob>,
    fixture: &Fixture,
    provider: &follow_up_provider::Provider,
    receipt: &ChildFollowUpStarted,
) {
    let finalizing = resumed.finalize_run(Ok(fixture.output.clone()));
    tokio::pin!(finalizing);
    // The child's request is held, so finalization cannot return: the case fails
    // the moment it does, wherever in this wait that happens. Driving both at
    // once is what lets the state be read at all — finalization's own cancel
    // poll owns the store's one connection while it waits, so a parked finalizer
    // would leave the read with nothing to acquire.
    let state = tokio::select! {
        biased;
        done = &mut finalizing => panic!(
            "root finalization returned while the accepted late child was unfinished: {done:?}"
        ),
        state = job_state(&fixture.store, resumed.job_id()) => state,
    };
    provider
        .release
        .store(true, std::sync::atomic::Ordering::Release);
    tokio::time::timeout(SETTLE_CEILING, &mut finalizing)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state,
        JobState::Running,
        "accepted unfinished child cannot coexist with root Paused"
    );
    assert!(resumed.active.lock().await.is_none());
    let history = fixture
        .store
        .list_coding_completion_history(resumed.job_id(), None, 32)
        .await
        .unwrap();
    assert_eq!(history.items.len(), 2);
    assert!(
        history
            .items
            .iter()
            .any(|item| item.result.invocation_id() == &receipt.invocation_id)
    );
    assert_eq!(
        job_state(&fixture.store, resumed.job_id()).await,
        JobState::Completed
    );
}
