//! The direct-mode root's stall bound counts only calls that FAILED, were
//! REFUSED or were DENIED: a successful call resets the streak, however little
//! it changed the job. So a root that keeps writing is never stopped, a run of
//! successful read-only calls is never stopped either, and a root that only
//! fails is stopped after `DIRECT_STALL_LIMIT` (96) consecutive failures.

use super::convergence::{STALL_AFTER, root_requests, tool_results};
use super::direct_task::{direct_job_without_progress_guards, root_checkout, tool_result};
use super::provider::Provider;
use saya_agent::{AgentOutput, NoopEventSink};
use saya_cli::{CodingError, NativeCodingJob};
use std::{sync::Arc, time::Duration};

/// Consecutive failed, refused or denied root calls the owner tolerates, as
/// `convergence.rs` defines it.
const DIRECT_STALL_AFTER: usize = 96;

/// The run's own outcome; the caller asserts whether the owner stopped it.
async fn run(job: &Arc<NativeCodingJob>) -> Result<AgentOutput, CodingError> {
    tokio::time::timeout(Duration::from_secs(180), job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
}

/// The refused write's error text, which the model saw before the run ended.
fn refusal_message(provider: &Provider) -> String {
    let refusal = tool_result(provider, "call-refused-write");
    refusal["error"]
        .as_str()
        .unwrap_or_else(|| panic!("the write outside the ceiling is refused: {refusal}"))
        .to_owned()
}

#[tokio::test]
async fn a_successful_root_write_does_not_end_the_run() {
    // 40 read-only calls, one successful write, then 57 more read-only calls.
    // Every one of them succeeds, so no run of failures is ever built up: the
    // run outlasts the 96-call bound and is never stopped.
    let (fixture, _app, job, provider) = direct_job_without_progress_guards(
        "direct-progress-write",
        81,
        vec!["added.txt".into()],
        vec![],
    )
    .await;

    run(&job)
        .await
        .expect("a root whose calls all succeed is not stalled");

    assert!(job.failure().is_none(), "{:?}", job.failure());
    assert!(
        root_requests(&provider) > DIRECT_STALL_AFTER,
        "the run outlasts the direct bound, so only success can have reset the count: {} requests",
        root_requests(&provider)
    );
    assert_eq!(
        std::fs::read_to_string(root_checkout(&fixture, &job).await.join("added.txt")).unwrap(),
        "FIXED\n",
        "the write landed in the root working checkout"
    );
}

#[tokio::test]
async fn a_refused_root_write_does_not_stall_a_succeeding_root() {
    // 95 read-only calls, a write the root's authority refuses, then one more
    // read. The refused write is one failure; the successful reads around it
    // reset the streak, so the run ends with its own answer rather than a stop.
    let (fixture, _app, job, provider) = direct_job_without_progress_guards(
        "direct-progress-refused",
        67,
        vec!["added.txt".into()],
        vec![],
    )
    .await;

    run(&job)
        .await
        .expect("one refused write between successful reads is not a stall");

    assert!(job.failure().is_none(), "{:?}", job.failure());
    // There is no job-level write-path ceiling, so the refusal is the
    // write authority's own digest precondition: the guarded replace's digest
    // is not the file's, so nothing was written.
    assert!(
        refusal_message(&provider).contains("the file changed since it was observed"),
        "the refused write was refused by the root's write authority: {}",
        refusal_message(&provider)
    );
    assert_eq!(
        std::fs::read_to_string(root_checkout(&fixture, &job).await.join("tracked.txt")).unwrap(),
        "original\n",
        "the refused write changed nothing"
    );
}

#[tokio::test]
async fn a_direct_root_that_keeps_writing_is_not_stalled_past_36_calls() {
    // Forty-five successful writes, then an answer. Every write succeeds, so
    // no run of failures reaches the bound.
    let paths = (0..45)
        .map(|index| format!("file-{index:02}.txt"))
        .collect();
    let (fixture, _app, job, provider) =
        direct_job_without_progress_guards("direct-progress-writes", 83, paths, vec![]).await;

    run(&job)
        .await
        .expect("a root that keeps writing is never stalled");

    assert!(job.failure().is_none(), "{:?}", job.failure());
    assert!(
        root_requests(&provider) > STALL_AFTER,
        "the write session outlasts the 36-call mid-point: {} requests",
        root_requests(&provider)
    );
    let working = root_checkout(&fixture, &job).await;
    for name in ["file-00.txt", "file-44.txt"] {
        assert_eq!(
            std::fs::read_to_string(working.join(name)).unwrap(),
            format!("root wrote {}\n", name[5..7].parse::<usize>().unwrap()),
            "{name} landed in the root working checkout"
        );
    }
}

/// A hundred and twenty consecutive successful calls that change nothing are
/// NOT a stall: only failed, refused or denied calls count. Against the code
/// the revision-based count would stop this run at 96.
#[tokio::test]
async fn a_root_of_successful_read_only_calls_is_never_stalled() {
    let (_fixture, _app, job, provider) = direct_job_without_progress_guards(
        "direct-progress-reads",
        84,
        vec!["added.txt".into()],
        vec![],
    )
    .await;
    run(&job)
        .await
        .expect("successful read-only calls are never a stall");
    assert!(job.failure().is_none(), "{:?}", job.failure());
    assert!(
        root_requests(&provider) > DIRECT_STALL_AFTER,
        "the run outlasts the old bound: {} requests",
        root_requests(&provider)
    );
    assert!(
        tool_results(&provider).contains_key("read-119"),
        "all 120 successful reads ran"
    );
}

/// Ninety-six consecutive refused calls still end the run with
/// `CoordinatorStalled`. The bound is unchanged; only what counts toward it is.
#[tokio::test]
async fn ninety_six_consecutive_refused_calls_stall_the_root() {
    let (_fixture, _app, job, provider) = direct_job_without_progress_guards(
        "direct-progress-refused-stall",
        66,
        vec!["tracked.txt".into()],
        vec![],
    )
    .await;
    let error = run(&job)
        .await
        .expect_err("96 consecutive refused calls stall the root");
    assert!(error.to_string().contains("coordinator_stalled"), "{error}");
    let requests = root_requests(&provider);
    assert!(
        (DIRECT_STALL_AFTER..=DIRECT_STALL_AFTER + 2).contains(&requests),
        "{requests} requests; the bound is {DIRECT_STALL_AFTER} consecutive failed or refused calls"
    );

    // 95 refused calls then an answer: one short of the bound is fine.
    let (_fixture, _app, job, provider) = direct_job_without_progress_guards(
        "direct-progress-refused-short",
        85,
        vec!["added.txt".into()],
        vec![],
    )
    .await;
    run(&job)
        .await
        .expect("95 successful calls are short of the 96-call bound");
    assert!(job.failure().is_none(), "{:?}", job.failure());
    assert_eq!(
        root_requests(&provider),
        96,
        "95 calls plus the answering request"
    );
}
