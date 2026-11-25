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
    sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError, Weak},
};

use super::in_place_lease::{InPlaceLease, LeaseCell};

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
/// session the lease names, or the job that took it.
///
/// A folder is not "another session's" just because its lease is held: a
/// session's own run leases the folder for as long as it is open, and a job
/// that states that session is admitted beside it. A run's shape reads this to
/// tell the session's own folder from a stranger's.
pub(crate) fn lease_owner(root_id: &str) -> Option<String> {
    lock(&LEASED).get(root_id).cloned()
}

/// One job admitted beside a session's lease, and the cell that would receive
/// the lease if the session ends while the job still runs. The cell is weak: a
/// job that goes away is not a place to hand a folder to.
struct Dependent {
    job: String,
    cell: Weak<Mutex<Option<InPlaceLease>>>,
}

/// The jobs this process admitted beside a session's lease, by root id.
static DEPENDENTS: LazyLock<Mutex<BTreeMap<String, Vec<Dependent>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn lock_dependents() -> MutexGuard<'static, BTreeMap<String, Vec<Dependent>>> {
    DEPENDENTS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Records that `job` runs beside `root_id`'s lease and may inherit it when the
/// lease's session ends. `cell` is the job's own lease slot, so the handover
/// binds the authorized value — the job this process admitted — and never a
/// second read of the folder's owner.
pub(super) fn dependent(root_id: &str, job: &str, cell: &LeaseCell) {
    lock_dependents()
        .entry(root_id.to_owned())
        .or_default()
        .push(Dependent {
            job: job.to_owned(),
            cell: Arc::downgrade(cell),
        });
}

/// Hands `lease` to the first live dependent of `root_id`, atomically: the
/// descriptor is moved into the job's slot, never closed and reopened, so the
/// folder has a holder at every instant. A job whose slot no longer holds its
/// own non-holding lease has given the folder up, and the next one is asked.
/// Reports whether a job took the lease.
pub(super) fn hand_over(root_id: &str, lease: InPlaceLease) -> bool {
    let mut lease = Some(lease);
    while let Some((job, cell)) = take_dependent(root_id) {
        let mut slot = cell.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.as_ref().is_some_and(InPlaceLease::is_dependent)
            && let Some(mut adopted) = lease.take()
        {
            // The job now holds the folder and may hand it onward in turn.
            adopted.adopt_handover();
            *slot = Some(adopted);
            drop(slot);
            leased(root_id, &job);
            return true;
        }
    }
    false
}

/// The next registered dependent of `root_id` whose job still lives, removed
/// from the registry as it is taken.
fn take_dependent(root_id: &str) -> Option<(String, LeaseCell)> {
    let mut map = lock_dependents();
    loop {
        let next = map.get_mut(root_id)?.pop();
        let Some(dependent) = next else {
            map.remove(root_id);
            return None;
        };
        if let Some(cell) = dependent.cell.upgrade() {
            return Some((dependent.job, cell));
        }
    }
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
