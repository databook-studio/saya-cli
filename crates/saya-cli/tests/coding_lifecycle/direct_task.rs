//! Direct-mode `helpers` tool lifecycle: the root hands independent pieces to
//! helper children synchronously; every helper works in the root's own working
//! checkout W, so there is no per-helper checkout and no merge — the
//! helper's own writes are already in the checkout the operator reviews. The
//! tool result returns a redacted, authority-free summary per child. This file
//! holds the shared fixtures and the summary / retained-W tests; the join-side
//! behaviours live in `direct_task_join`.

use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use serde_json::Value;
use std::{path::PathBuf, sync::Arc, time::Duration};

pub(crate) fn tool_result(provider: &Provider, id: &str) -> Value {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .flat_map(|body| {
            let request: Value = serde_json::from_str(body).unwrap();
            request["messages"].as_array().cloned().unwrap_or_default()
        })
        .find(|message| message["tool_call_id"] == id)
        .and_then(|message| serde_json::from_str(message["content"].as_str()?).ok())
        .unwrap_or(Value::Null)
}

pub(crate) async fn root_checkout(fixture: &Fixture, job: &saya_cli::NativeCodingJob) -> PathBuf {
    fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 100)
        .await
        .unwrap()
        .items
        .iter()
        // A prepared candidate is agent-less too, so the root's own working
        // checkout is the one the job derives from its id (`work-…`).
        .find(|record| {
            record
                .ownership
                .managed_checkout_id()
                .as_str()
                .starts_with("work-")
        })
        .map(|record| PathBuf::from(record.ownership.managed_path().as_str()))
        .expect("the root working checkout W exists")
}

/// The advisory readiness the job reports — the `saya coding show`
/// surface. The retired `coding_status` tool has no replacement, so a test
/// asserts the facts here (after the run, or from a task running beside it).
pub(crate) async fn readiness_now(job: &saya_cli::NativeCodingJob) -> Value {
    job.status()
        .await
        .unwrap()
        .get("readiness")
        .cloned()
        .unwrap_or(Value::Null)
}

/// Opens a direct-mode job without running it: a scripted provider at `mode`,
/// the approved ceiling `write_paths`, and optional transport retry delays. The
/// logical-request backstop is stated as 400 so a long scripted session (a
/// hundred-plus root calls) fits; the scenarios that make a handful of calls
/// are unaffected.
pub(crate) async fn direct_job(
    label: &str,
    mode: u8,
    write_paths: Vec<String>,
    retry_delays: Vec<u64>,
) -> (
    Fixture,
    Arc<CodingApplication>,
    Arc<saya_cli::NativeCodingJob>,
    Provider,
) {
    direct_job_with_guards(label, mode, write_paths, retry_delays, true).await
}

/// The same job with both progress guards disabled. A scenario that scripts a
/// long run of deliberately identical calls is testing the owner's stall
/// bound, not the job's progress policy, so it turns the policy off.
pub(crate) async fn direct_job_without_progress_guards(
    label: &str,
    mode: u8,
    write_paths: Vec<String>,
    retry_delays: Vec<u64>,
) -> (
    Fixture,
    Arc<CodingApplication>,
    Arc<saya_cli::NativeCodingJob>,
    Provider,
) {
    direct_job_with_guards(label, mode, write_paths, retry_delays, false).await
}

async fn direct_job_with_guards(
    label: &str,
    mode: u8,
    write_paths: Vec<String>,
    retry_delays: Vec<u64>,
    progress_guards: bool,
) -> (
    Fixture,
    Arc<CodingApplication>,
    Arc<saya_cli::NativeCodingJob>,
    Provider,
) {
    let provider = Provider::script(false, mode);
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    let mut runtime = fixture.runtime.clone();
    runtime.resolved.limits.job.logical_requests = finite(400);
    if !progress_guards {
        runtime.resolved.limits.job.no_progress_turns = 0;
        runtime.resolved.limits.job.repeat_limit = 0;
    }
    runtime.resolved.ai.retry_delays_ms = retry_delays;
    let application = CodingApplication::open(
        runtime,
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Direct-mode task work with helpers".into(),
        write_paths,
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    (fixture, application, job, provider)
}

/// Runs a direct-mode job to completion: a scripted provider at `mode`, the
/// approved ceiling `write_paths`, and optional transport retry delays.
pub(crate) async fn run_direct(
    label: &str,
    mode: u8,
    write_paths: Vec<String>,
    retry_delays: Vec<u64>,
) -> (
    Fixture,
    Arc<CodingApplication>,
    Arc<saya_cli::NativeCodingJob>,
    Provider,
) {
    let (fixture, application, job, provider) =
        direct_job(label, mode, write_paths, retry_delays).await;
    tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .unwrap();
    (fixture, application, job, provider)
}

#[tokio::test]
async fn a_direct_root_hands_a_piece_to_a_child_and_gets_its_summary() {
    let (fixture, _app, job, provider) = run_direct(
        "direct-task-summary",
        62,
        vec!["file-a.txt".into(), "file-b.txt".into()],
        vec![],
    )
    .await;
    let summary = &tool_result(&provider, "call-task")[0];
    assert_eq!(summary["agent"], "child-a", "{summary}");
    assert_eq!(summary["outcome"], "completed", "{summary}");
    assert_eq!(summary["authority"], "none", "{summary}");
    assert!(
        summary["summary"].as_str().is_some_and(|t| !t.is_empty()),
        "{summary}"
    );
    // The helper writes W directly, so its work is already in the checkout
    // the summary names: there is no per-helper delta to attribute.
    let working = root_checkout(&fixture, &job).await;
    assert_eq!(
        std::fs::read_to_string(working.join("file-a.txt")).unwrap(),
        "child-a wrote\n"
    );
}

#[tokio::test]
async fn the_root_keeps_working_after_a_helper_finishes() {
    let (fixture, _app, job, _provider) = run_direct(
        "direct-task-retained",
        62,
        vec!["file-a.txt".into(), "file-b.txt".into()],
        vec![],
    )
    .await;
    let working = root_checkout(&fixture, &job).await;
    assert_eq!(
        std::fs::read_to_string(working.join("file-a.txt")).unwrap(),
        "child-a wrote\n"
    );
    assert_eq!(
        std::fs::read_to_string(working.join("file-b.txt")).unwrap(),
        "root wrote after the merge\n"
    );
}

fn finite(value: u64) -> saya_config::BudgetLimit {
    saya_config::BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}
