//! The owner's stall bound counts only consecutive failed, refused or denied
//! root calls (`DIRECT_STALL_AFTER`). A root whose calls keep succeeding is
//! never stopped, however little they changed the job: the direct root's
//! status and prepare calls are plain reads, so a
//! root that repeats them runs to its own answer and the owner prepares the
//! candidate at the end. (The failing-root side of the bound is
//! `direct_progress`.)
use super::{
    DIRECT_STALL_AFTER, child_checkouts, direct_fixture, root_requests, tool_results, write_job,
};
use crate::provider::Provider;
use saya_agent::NoopEventSink;
use saya_types::JobState;
use serde_json::json;
use std::{fs, time::Duration};

/// The mode-91 run's root turns before it answers: one helper, one read, then
/// a long run of repeated reads.
const MODE_91_TURNS: usize = 121;

#[tokio::test]
async fn a_root_that_keeps_reading_without_progress_is_never_stalled() {
    // Mode 91: the root hands a piece to a helper, reads the checkout, then
    // repeats a read over and over. Every call succeeds, so no run of failures
    // is ever built up and the run outlasts the bound.
    let provider = Provider::script(false, 91);
    let fixture = direct_fixture("stall-reads", &provider).await;
    let job = write_job(&fixture, "stall-reads", &["tracked.txt"]).await;

    tokio::time::timeout(Duration::from_secs(240), job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .expect("a root whose calls keep succeeding is never stopped");

    assert!(job.failure().is_none(), "{:?}", job.failure());
    let requests = root_requests(&provider);
    assert_eq!(
        requests, MODE_91_TURNS,
        "the run made every call and then answered"
    );
    assert!(
        requests > DIRECT_STALL_AFTER,
        "{requests} requests outlast the {DIRECT_STALL_AFTER}-call bound, so only success can have kept it alive"
    );
    let results = tool_results(&provider);
    assert!(
        results["status-ready"].get("error").is_none(),
        "the read the retired status call became succeeds: {}",
        results["status-ready"]
    );
    assert!(
        results["prepare-3"].get("error").is_none(),
        "the repeated read the retired prepare call became succeeds: {}",
        results["prepare-3"]
    );

    // The owner prepares at the end of the run: the child's change is a ready
    // candidate.
    let readiness = crate::direct_task::readiness_now(&job).await;
    assert_eq!(readiness["state"], "candidate_ready", "{readiness}");
    assert_eq!(readiness["candidate_ready"], true, "{readiness}");
    assert_eq!(
        readiness["changed_paths"],
        json!([{"kind": "modified", "path": "tracked.txt"}]),
        "{readiness}"
    );
}

#[tokio::test]
async fn the_end_of_run_preparation_leaves_the_candidate_and_effect_accounting_complete() {
    // The same run: it was not stopped, so the owner's end-of-run hook ran
    // exactly once, the helper's change is in W, and nothing is left uncertain.
    let provider = Provider::script(false, 91);
    let fixture = direct_fixture("stall-accounting", &provider).await;
    let job = write_job(&fixture, "stall-accounting", &["tracked.txt"]).await;

    tokio::time::timeout(Duration::from_secs(240), job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .expect("a root whose calls keep succeeding is never stopped");

    assert_eq!(
        crate::direct_prepare::fixture::checkouts_named(&fixture, &job, "candidate-").await,
        1,
        "the owner prepared exactly one candidate at the end of the run"
    );
    assert_eq!(
        child_checkouts(&fixture, &job).await,
        0,
        "helpers share W, so no per-helper checkout exists"
    );
    let records = fixture.root.join("coding").join(job.job_id().as_str());
    let receipts = fs::read_dir(records)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("prepared-candidate-")
        })
        .count();
    assert_eq!(receipts, 1, "the prepared receipt was written once");
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations,
        0,
        "no failed or refused call left an effect uncertain"
    );
    let working = crate::direct_task::root_checkout(&fixture, &job).await;
    assert_eq!(
        fs::read_to_string(working.join("tracked.txt")).unwrap(),
        "FIXED\n",
        "the helper's change is in the working checkout the candidate was built from"
    );
}

#[tokio::test]
async fn a_root_that_keeps_succeeding_between_long_polls_is_never_stalled() {
    // Mode 92: two long runs of read-only polling, each ended by a helper call
    // that changes the job. Every call succeeds, so only a bound that forgot to
    // reset on success could stop it.
    let provider = Provider::script(false, 92);
    let fixture = direct_fixture("stall-progress", &provider).await;
    let job = write_job(&fixture, "stall-progress", &["new-a.txt", "new-b.txt"]).await;

    tokio::time::timeout(Duration::from_secs(240), job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .expect("a root whose calls keep succeeding is never stopped");

    assert!(job.failure().is_none(), "{:?}", job.failure());
    assert!(
        root_requests(&provider) > DIRECT_STALL_AFTER,
        "the run outlasts the bound, so only success can have reset the count"
    );
    let results = tool_results(&provider);
    assert!(
        results.contains_key("poll-120"),
        "the second long poll run ran to its end"
    );
    assert_eq!(
        fixture
            .store
            .get_coding_job(job.job_id())
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Completed
    );
}
