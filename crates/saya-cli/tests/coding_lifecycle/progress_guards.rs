//! R10: a job stops on its own progress policy, not on a request or token
//! count. Identical calls stop as `repeating`, eight turns without progress
//! stop as `no_progress` and still prepare the candidate, and a long healthy
//! run is never stopped.

use super::convergence::{root_requests, tool_results};
use super::direct_prepare::fixture::checkout_path;
use super::direct_task::direct_job;
use saya_agent::{AgentOutput, NoopEventSink};
use saya_cli::{CodingError, NativeCodingJob};
use std::{sync::Arc, time::Duration};

/// The run's own outcome; a clean progress stop is `Ok`.
async fn run(job: &Arc<NativeCodingJob>) -> Result<AgentOutput, CodingError> {
    tokio::time::timeout(Duration::from_secs(180), job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
}

#[tokio::test]
async fn identical_reads_stop_the_job_as_repeating() {
    let (_fixture, _app, job, provider) = direct_job(
        "progress-repeating",
        115,
        vec!["tracked.txt".into()],
        vec![],
    )
    .await;

    run(&job)
        .await
        .expect("a clean progress stop returns the model's final output");

    let failure = job.failure().expect("the repeat guard latched a failure");
    assert_eq!(failure.class.as_str(), "repeating");
    assert_eq!(failure.limit.as_deref(), Some("limits.job.repeat_limit"));
    assert_eq!(failure.limit_value, Some(3));
    assert_eq!(failure.measured, Some(4), "the fourth identical result");
    assert!(
        failure
            .message
            .as_deref()
            .is_some_and(|message| message.contains("limits.job.repeat_limit")),
        "the message names the key to change: {:?}",
        failure.message
    );
    assert_eq!(
        root_requests(&provider),
        5,
        "four identical reads plus the final turn"
    );
}

#[tokio::test]
async fn eight_no_progress_turns_stop_the_job_and_still_prepare_the_candidate() {
    let (fixture, app, job, provider) =
        direct_job("progress-none", 116, vec!["added.txt".into()], vec![]).await;

    let output = run(&job)
        .await
        .expect("a clean progress stop returns the model's final output");

    let failure = job
        .failure()
        .expect("the no-progress guard latched a failure");
    assert_eq!(failure.class.as_str(), "no_progress");
    assert_eq!(
        failure.limit.as_deref(),
        Some("limits.job.no_progress_turns")
    );
    assert_eq!(failure.limit_value, Some(8));
    assert_eq!(failure.measured, Some(8));
    assert!(
        failure
            .message
            .as_deref()
            .is_some_and(|message| message.contains("limits.job.no_progress_turns")),
        "the message names the key to change: {:?}",
        failure.message
    );
    assert_eq!(
        root_requests(&provider),
        10,
        "one write, eight failed reads, and the final turn"
    );

    // The turn that tripped the guard is recorded against its limit, so an
    // operator reading `saya job show` sees how close the run was.
    let status = app.status(job.job_id()).await.unwrap();
    let check = &status["progress"]["check"];
    assert_eq!(check["turns_without_progress"], 8, "{status}");
    assert_eq!(check["limit"], 8, "{status}");
    assert_eq!(check["reset"], false, "{status}");
    assert!(
        check["message"]
            .as_str()
            .is_some_and(|message| message.contains("no progress for 8 of 8 turns")),
        "{status}"
    );
    // The same check is a durable job event, so `saya job events` sees it.
    let events = fixture
        .store
        .list_coding_events(job.job_id(), None, 100)
        .await
        .unwrap();
    assert!(
        events.events.iter().any(|event| matches!(
            event.event(),
            saya_types::CodingEventKind::ProgressReported {
                turns_without_progress: 8,
                limit: Some(8),
                reset: false,
                ..
            }
        )),
        "the guard's check is a durable job event: {:?}",
        events.events
    );

    // The job ended cleanly: the owner prepared the candidate from the one
    // successful write, exactly as it does for a normal finish.
    let envelope: serde_json::Value =
        serde_json::from_str(&output.answer).expect("the run's answer is the readiness envelope");
    assert_eq!(envelope["candidate_ready"], true, "{envelope}");
    let candidate = checkout_path(
        &fixture,
        &job,
        envelope["candidate"]
            .as_str()
            .unwrap_or_else(|| panic!("the owner prepared a candidate: {envelope}")),
    )
    .await;
    assert_eq!(
        std::fs::read_to_string(candidate.join("added.txt")).unwrap(),
        "FIXED\n",
        "the candidate carries the write the run made before it stopped"
    );
}

// The 120 productive calls are `shell` commands in the sandboxed lane, which
// only macOS proves. The Linux counterpart at the end of this module pins the
// host-lane refusal instead.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_long_productive_run_of_120_requests_is_not_stopped() {
    let (_fixture, _app, job, provider) =
        direct_job("progress-productive", 117, vec!["added.txt".into()], vec![]).await;

    run(&job)
        .await
        .expect("a productive run finishes on its own answer");

    assert!(job.failure().is_none(), "{:?}", job.failure());
    assert!(
        root_requests(&provider) >= 120,
        "the run made its 120 productive calls: {} requests",
        root_requests(&provider)
    );
    assert!(
        tool_results(&provider).contains_key("echo-119"),
        "all 120 distinct commands ran"
    );
}

// The Linux counterpart: the sandboxed lane is not offered, so a `shell` call
// is refused on the host lane and never counts as progress. A run built from
// commands therefore stops on the no-progress guard rather than running them.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_host_lane_shell_attempt_is_refused_and_never_counts_as_progress() {
    let (_fixture, _app, job, provider) =
        direct_job("progress-host-lane", 117, vec!["added.txt".into()], vec![]).await;

    run(&job)
        .await
        .expect("a clean progress stop returns the model's final output");

    let failure = job
        .failure()
        .expect("the no-progress guard latched a failure");
    assert_eq!(failure.class.as_str(), "no_progress");
    let results = tool_results(&provider);
    let first = results
        .get("echo-0")
        .expect("the first shell attempt is recorded");
    assert!(
        first["error"]
            .as_str()
            .is_some_and(|error| error.contains("no operator is available")),
        "the sandboxed lane is not offered on Linux: {first}"
    );
}
