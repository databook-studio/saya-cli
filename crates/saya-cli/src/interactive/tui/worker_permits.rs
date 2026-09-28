//! The process-wide cap on background query workers.
//!
//! A direct-SQL command or an `/investigation run` replay runs on its own
//! worker thread, and Esc detaches it by dropping the receiver: the worker
//! keeps running — possibly for a long time — with nothing left tracking it
//! on `App`. The only fact every such worker shares, attached or detached,
//! is the process, so the bound lives here: at most
//! [`MAX_BACKGROUND_WORKERS`] worker threads run at once, each holding one
//! RAII permit from just before its spawn until its thread function
//! returns. Dropping the receiver does not release a permit; the worker's
//! own exit does — normally, on error, or on panic (unwinding runs `Drop`).
//!
//! Admission is refusal, not queueing: when every permit is held, a
//! dispatched query is refused with [`CAP_REFUSAL`] and the UI stays
//! responsive.

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Hard cap on concurrently running background query workers — direct-SQL
/// tasks and investigation replays together, attached or detached.
pub(crate) const MAX_BACKGROUND_WORKERS: usize = 4;

/// The refusal when every permit is held. Admission fails only when all
/// four are running, so the count in the message is the cap itself (kept in
/// step by the compile-time assertion below).
pub(crate) const CAP_REFUSAL: &str = "4 queries are still running in the background (including detached ones); \
     wait for one to finish.";

const _: () = assert!(
    MAX_BACKGROUND_WORKERS == 4,
    "CAP_REFUSAL names the cap; keep the message and the constant in step"
);

/// The process-wide permit counter — the one shared fact about workers the
/// `App` no longer tracks.
fn shared_counter() -> &'static Arc<AtomicUsize> {
    static COUNTER: OnceLock<Arc<AtomicUsize>> = OnceLock::new();
    COUNTER.get_or_init(|| Arc::new(AtomicUsize::new(0)))
}

/// Acquires one permit from the process-wide pool, or `None` when all
/// [`MAX_BACKGROUND_WORKERS`] are held. Call it **before** spawning and
/// move the permit into the worker thread.
pub(crate) fn try_acquire_worker_permit() -> Option<WorkerPermit> {
    acquire_on(shared_counter())
}

/// How many background workers hold a permit right now — the running
/// queries the cap counts, attached or detached. Production reads nothing
/// (the cap is enforced at admission); this is the tests' window on the
/// count.
#[cfg(test)]
pub(crate) fn running_workers() -> usize {
    shared_counter().load(Ordering::Acquire)
}

/// A held permit: one worker's claim on a background slot. Dropping it —
/// when the thread function returns, unwinding included — releases the
/// slot, so the guard travels into the worker thread and never out of it.
pub(crate) struct WorkerPermit {
    counter: Arc<AtomicUsize>,
}

impl WorkerPermit {
    /// A permit standing alone on its own counter, for tests that spawn a
    /// worker off the admission path (completion and detach behaviour, not
    /// the cap) and must not touch the process-wide pool.
    #[cfg(test)]
    pub(crate) fn for_tests() -> WorkerPermit {
        acquire_on(&Arc::new(AtomicUsize::new(0))).expect("a fresh counter admits")
    }
}

impl Drop for WorkerPermit {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Release);
    }
}

/// The one acquisition implementation: a CAS loop so concurrent takers
/// never push the count past the cap.
fn acquire_on(counter: &Arc<AtomicUsize>) -> Option<WorkerPermit> {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        if current >= MAX_BACKGROUND_WORKERS {
            return None;
        }
        match counter.compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                return Some(WorkerPermit {
                    counter: Arc::clone(counter),
                });
            }
            Err(observed) => current = observed,
        }
    }
}

/// Serializes the tests that touch the process-wide pool, so one test's
/// permits never race another's held phase. Sync tests take it with
/// [`test_permit_lock`]; an async test takes the same lock with
/// [`test_permit_lock_async`] and may hold the guard across its awaits.
#[cfg(test)]
pub(crate) fn test_permit_lock() -> tokio::sync::MutexGuard<'static, ()> {
    permit_lock().blocking_lock()
}

#[cfg(test)]
pub(crate) async fn test_permit_lock_async() -> tokio::sync::MutexGuard<'static, ()> {
    permit_lock().lock().await
}

#[cfg(test)]
fn permit_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::const_new(()))
}
