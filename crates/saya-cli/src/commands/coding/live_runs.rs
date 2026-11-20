//! The liveness facts this process holds and no store row does.
//!
//! The store's coding tables are durable facts: an epoch, a lease file's
//! existence, a job's state. None of them says whether a run is executing or
//! between its turns right now, and that is the one fact an outside writer must
//! respect, because a live run numbers its own events from memory and the store
//! refuses any append that does not take the number it is about to take.
//!
//! Two facts are kept here, both process-local: the folders this process
//! currently leases, and the runs it currently holds open. A verb asks them
//! read-only, before it asks a human or appends anything.

use std::{
    collections::BTreeSet,
    sync::{LazyLock, Mutex, PoisonError},
};

/// The in-place root ids whose lease this process holds.
static LEASED: LazyLock<Mutex<BTreeSet<String>>> = LazyLock::new(|| Mutex::new(BTreeSet::new()));

/// The job ids whose root run is open in this process.
static OPEN_RUNS: LazyLock<Mutex<BTreeSet<String>>> = LazyLock::new(|| Mutex::new(BTreeSet::new()));

fn lock(
    set: &'static LazyLock<Mutex<BTreeSet<String>>>,
) -> std::sync::MutexGuard<'static, BTreeSet<String>> {
    set.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Records that this process now holds `root_id`'s lease.
pub(super) fn leased(root_id: &str) {
    lock(&LEASED).insert(root_id.to_owned());
}

/// Records that this process gave `root_id`'s lease up.
pub(super) fn released(root_id: &str) {
    lock(&LEASED).remove(root_id);
}

/// Whether this process itself holds `root_id`'s lease: the holder of a lease
/// file is then a job object of ours, not another process's run.
pub(super) fn leased_here(root_id: &str) -> bool {
    lock(&LEASED).contains(root_id)
}

/// Records that `job`'s root run opened: its composition and its own event
/// numbering are held until the run closes.
pub(super) fn run_opened(job: &str) {
    lock(&OPEN_RUNS).insert(job.to_owned());
}

/// Records that `job`'s root run closed.
pub(super) fn run_closed(job: &str) {
    lock(&OPEN_RUNS).remove(job);
}

/// Whether `job`'s root run is open in this process right now.
pub(super) fn run_is_open(job: &str) -> bool {
    lock(&OPEN_RUNS).contains(job)
}
