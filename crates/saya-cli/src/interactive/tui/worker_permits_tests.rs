//! The process-wide background-worker cap, pinned without the deleted app.
//!
//! Every running query worker — attached or detached — holds one of the four
//! permits from just before its spawn until its worker thread really exits.
//! Admission refuses when all are held, a released permit is acquirable again,
//! and a panicking worker releases its permit by unwinding. These are the pins
//! the old surface wrote through its `App`; the mechanism never lived there, so
//! they moved down onto the pool itself.

use super::worker_permits::{
    MAX_BACKGROUND_WORKERS, WorkerPermit, running_workers, test_permit_lock,
    try_acquire_worker_permit,
};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

/// Waits until the shared counter reads `count` — a permit is released when its
/// worker thread really exits, which the test observes instead of guessing
/// timing.
fn wait_for_running(count: usize, why: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while running_workers() != count {
        assert!(Instant::now() < deadline, "never settled: {why}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A worker body gated behind a release signal: it blocks until the sender
/// sends (or drops), so the test controls exactly when the thread — and with it
/// the permit — goes away.
fn gated_worker() -> (Sender<()>, impl FnOnce() + Send + 'static) {
    let (release, release_rx) = mpsc::channel::<()>();
    (release, move || {
        let _ = release_rx.recv();
    })
}

// -- the lock hands over a drained pool --------------------------------------

/// The seam invariant that deflakes the cap tests: acquiring the test permit
/// lock must hand the caller a pool with **no stale permits**. A permit
/// acquired under the lock can outlive its holder's guard — a worker thread
/// finishing after the test observed its completion — so the acquirer must not
/// take the lock until that permit is gone.
#[test]
fn the_permit_lock_hands_over_a_drained_pool() {
    let (handoff_tx, handoff_rx) = mpsc::channel();
    let holder = std::thread::spawn(move || {
        // The stale shape: a real permit acquired under the lock that outlives
        // the lock handoff, like a worker thread still finishing.
        let permit = {
            let _guard = test_permit_lock();
            try_acquire_worker_permit().expect("the pool admits under the lock")
        };
        handoff_tx
            .send(())
            .expect("the test waits for the handoff signal");
        std::thread::sleep(Duration::from_millis(100));
        drop(permit);
    });
    handoff_rx
        .recv()
        .expect("the holder signals: lock free, permit still held");

    let _serialized = test_permit_lock();
    assert_eq!(
        running_workers(),
        0,
        "the lock must hand over a drained pool: a previous holder's permit outlived the handoff"
    );
    holder.join().expect("the holder thread exits");
}

// -- the pool ---------------------------------------------------------------

/// The cap admits up to four holders, refuses the fifth, and a released permit
/// is acquirable again — no queueing, no over-admission.
#[test]
fn acquiring_holds_up_to_the_cap_then_refuses_and_release_restores() {
    let _serialized = test_permit_lock();
    assert_eq!(running_workers(), 0, "the lock guarantees an empty pool");
    let mut permits: Vec<WorkerPermit> = (0..MAX_BACKGROUND_WORKERS)
        .filter_map(|_| try_acquire_worker_permit())
        .collect();
    assert_eq!(
        permits.len(),
        MAX_BACKGROUND_WORKERS,
        "admission under the cap succeeds"
    );
    assert_eq!(running_workers(), MAX_BACKGROUND_WORKERS);
    assert!(
        try_acquire_worker_permit().is_none(),
        "the cap refuses when every permit is held"
    );
    drop(permits.pop());
    assert_eq!(running_workers(), MAX_BACKGROUND_WORKERS - 1);
    assert!(
        try_acquire_worker_permit().is_some(),
        "a released permit is acquirable again"
    );
}

// -- detached workers count against the cap ---------------------------------

/// A permit moved into a worker thread keeps counting while that thread runs,
/// with nothing tracking it on a surface — the detached shape — and its exit
/// frees the slot, so the refusal is never queued.
#[test]
fn a_detached_worker_holds_its_permit_until_its_thread_exits() {
    let _serialized = test_permit_lock();
    let mut releases = Vec::new();
    for _ in 0..MAX_BACKGROUND_WORKERS {
        let permit = try_acquire_worker_permit().expect("the pool admits under the cap");
        let (release, body) = gated_worker();
        std::thread::spawn(move || {
            let _held = permit;
            body();
        });
        releases.push(release);
    }
    wait_for_running(
        MAX_BACKGROUND_WORKERS,
        "detached workers still hold their permits",
    );
    assert!(
        try_acquire_worker_permit().is_none(),
        "the fifth worker is refused while all four run"
    );
    drop(releases.remove(0));
    wait_for_running(
        MAX_BACKGROUND_WORKERS - 1,
        "the released worker must free its permit",
    );
    assert!(
        try_acquire_worker_permit().is_some(),
        "admission succeeds once a detached worker exits"
    );
}

// -- panic release ----------------------------------------------------------

/// A permit is released when the worker thread really exits — including by
/// panicking: unwinding the thread function drops the permit guard, and
/// admission recovers.
#[test]
fn a_panicking_worker_releases_its_permit() {
    let _serialized = test_permit_lock();
    let permit = try_acquire_worker_permit().expect("the pool admits");
    assert_eq!(running_workers(), 1, "the held permit is counted");
    std::thread::spawn(move || {
        let _held = permit;
        panic!("the worker dies mid-query");
    });
    wait_for_running(0, "the panicking worker must release its permit");
    assert!(
        try_acquire_worker_permit().is_some(),
        "admission recovers after the panicking worker released its permit"
    );
}
