//! Tests for the process-wide background-worker cap: every running SQL task
//! or replay — attached or detached — holds one of the four permits from
//! just before its spawn until its worker thread really exits; admission
//! refuses when all are held, detach never releases, and a panicking worker
//! releases its permit by unwinding.

use super::application::SecondSqlDecision;
use super::application::tests_support::idle_app;
use super::replay_task::{ReplayDone, ReplayTask, spawn_with};
use super::types::App;
use super::worker_permits::{
    CAP_REFUSAL, MAX_BACKGROUND_WORKERS, WorkerPermit, running_workers, test_permit_lock,
    try_acquire_worker_permit,
};
use crate::cli::InvestigationCommand;
use crate::render::RenderFormat;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

// -- fixtures ---------------------------------------------------------------

fn replay_task(id: &str) -> ReplayTask {
    ReplayTask {
        id: id.to_string(),
        command: InvestigationCommand::Run {
            id: id.to_string(),
            connection: None,
            revalidate: false,
            report: None,
            rows: None,
            overwrite: false,
        },
        format: RenderFormat::Text,
    }
}

/// A worker body gated behind a release signal: it blocks on the channel
/// until the sender drops (or sends), so the test controls exactly when the
/// worker thread — and with it the permit — goes away.
fn gated_worker() -> (
    Sender<()>,
    impl FnOnce(&Sender<ReplayDone>) + Send + 'static,
) {
    let (release, release_rx) = mpsc::channel::<()>();
    (release, move |_tx: &Sender<ReplayDone>| {
        let _ = release_rx.recv();
    })
}

/// Puts a running replay on `app` the way the dispatch arm does: receiver,
/// task, dispatch instant — then detaches it through the real detach path,
/// so the worker keeps running with nothing tracking it on `App`.
fn start_and_detach(app: &mut App, rx: Receiver<ReplayDone>) {
    app.replay_task = Some((rx, replay_task("gated"), Instant::now()));
    app.detach_replay_task();
    assert!(
        app.replay_task.is_none(),
        "detach clears the tracked task; the worker keeps running"
    );
}

/// Admission on an idle app must hand back a permit, never refuse.
fn admitted_permit(app: &App) -> WorkerPermit {
    match app.admit_second_replay() {
        SecondSqlDecision::Start(permit) => permit,
        SecondSqlDecision::Reject(message) => panic!("admission must succeed: {message}"),
    }
}

/// Waits until the shared counter reads `count` — a worker's permit is
/// released when its thread really exits, which the test observes instead
/// of guessing timing.
fn wait_for_running(count: usize, why: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while running_workers() != count {
        assert!(Instant::now() < deadline, "never settled: {why}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

// -- the pool ---------------------------------------------------------------

/// The cap admits up to four holders, refuses the fifth, and a released
/// permit is acquirable again — no queueing, no over-admission.
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

/// Four start+detach rounds leave four gated workers running with nothing
/// tracked on `App`; the fifth admission is refused (the existing
/// one-attached-task rule is untouched — nothing is attached here).
/// Releasing one gate lets the worker exit and admission succeeds again.
#[test]
fn detached_workers_count_against_the_cap() {
    let _serialized = test_permit_lock();
    let mut app = idle_app();

    let mut releases = Vec::new();
    for _ in 0..MAX_BACKGROUND_WORKERS {
        let permit = admitted_permit(&app);
        let (release, body) = gated_worker();
        start_and_detach(&mut app, spawn_with(permit, body));
        releases.push(release);
    }

    assert_eq!(
        running_workers(),
        MAX_BACKGROUND_WORKERS,
        "detached workers still hold their permits"
    );
    match app.admit_second_replay() {
        SecondSqlDecision::Reject(message) => assert_eq!(
            message, CAP_REFUSAL,
            "the fifth admission is refused with the cap message"
        ),
        SecondSqlDecision::Start(_) => {
            panic!("a fifth background worker must be refused while all four run")
        }
    }
    match app.admit_second_sql() {
        SecondSqlDecision::Reject(message) => assert_eq!(
            message, CAP_REFUSAL,
            "the cap is one resource for SQL tasks and replays alike"
        ),
        SecondSqlDecision::Start(_) => {
            panic!("a SQL command must be refused while all four workers run")
        }
    }

    // Release one gate: the worker thread exits, its permit is released,
    // and admission succeeds again — the refusal is never queued.
    drop(releases.remove(0));
    wait_for_running(
        MAX_BACKGROUND_WORKERS - 1,
        "the released worker must free its permit",
    );
    assert!(
        matches!(app.admit_second_replay(), SecondSqlDecision::Start(_)),
        "admission succeeds once a detached worker exits"
    );
}

// -- panic release ----------------------------------------------------------

/// A permit is released when the worker thread really exits — including by
/// panicking: unwinding the thread function drops the permit guard, and
/// admission recovers.
#[test]
fn permit_released_on_worker_panic() {
    let _serialized = test_permit_lock();
    let app = idle_app();
    let permit = admitted_permit(&app);
    assert_eq!(running_workers(), 1, "the held permit is counted");

    let _rx = spawn_with(permit, |_tx: &Sender<ReplayDone>| {
        panic!("the worker dies mid-query");
    });

    wait_for_running(0, "the panicking worker must release its permit");
    assert!(
        matches!(app.admit_second_sql(), SecondSqlDecision::Start(_)),
        "admission recovers after the panicking worker released its permit"
    );
}
