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
    collections::{BTreeMap, BTreeSet},
    sync::{LazyLock, Mutex, PoisonError},
};

/// The in-place root ids whose lease this process holds, and the session each
/// one names. The holder is not always a job id: a root a session's run leases
/// is recorded under the session, so a job that session opens over the same
/// folder states a parent this map already names.
static LEASED: LazyLock<Mutex<BTreeMap<String, String>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// The job ids whose root run is open in this process.
static OPEN_RUNS: LazyLock<Mutex<BTreeSet<String>>> = LazyLock::new(|| Mutex::new(BTreeSet::new()));

fn lock(
    set: &'static LazyLock<Mutex<BTreeMap<String, String>>>,
) -> std::sync::MutexGuard<'static, BTreeMap<String, String>> {
    set.lock().unwrap_or_else(PoisonError::into_inner)
}

fn lock_runs(
    set: &'static LazyLock<Mutex<BTreeSet<String>>>,
) -> std::sync::MutexGuard<'static, BTreeSet<String>> {
    set.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Records that this process now holds `root_id`'s lease, naming `owner` — the
/// session the folder is leased for, or the job that leases it.
pub(super) fn leased(root_id: &str, owner: &str) {
    lock(&LEASED).insert(root_id.to_owned(), owner.to_owned());
}

/// Records that this process gave `root_id`'s lease up.
pub(super) fn released(root_id: &str) {
    lock(&LEASED).remove(root_id);
}

/// Whether this process itself holds `root_id`'s lease: the holder of a lease
/// file is then a job object of ours, not another process's run.
pub(super) fn leased_here(root_id: &str) -> bool {
    lock(&LEASED).contains_key(root_id)
}

/// The owner this process holds `root_id`'s lease for, if it holds it: the
/// session a job naming it is admitted as a dependent of, or the job itself.
pub(super) fn lease_owner(root_id: &str) -> Option<String> {
    lock(&LEASED).get(root_id).cloned()
}

/// Records that `job`'s root run opened: its composition and its own event
/// numbering are held until the run closes.
pub(super) fn run_opened(job: &str) {
    lock_runs(&OPEN_RUNS).insert(job.to_owned());
}

/// Records that `job`'s root run closed.
pub(super) fn run_closed(job: &str) {
    lock_runs(&OPEN_RUNS).remove(job);
}

/// Whether `job`'s root run is open in this process right now.
pub(super) fn run_is_open(job: &str) -> bool {
    lock_runs(&OPEN_RUNS).contains(job)
}
