//! Direct-mode `helpers` lifecycle, join-side behaviours: a failed child's
//! summary and the work a finished child leaves in the shared working checkout
//! W, and `helpers` refused and unadvertised for a job with no working
//! checkout. Every helper works in W directly, so there is no per-helper
//! checkout, no lease and no merge record to assert. The shared fixtures live
//! in `direct_task`.

use super::direct_task::{root_checkout, run_direct, tool_result};
use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use serde_json::Value;
use std::time::Duration;

#[tokio::test]
async fn a_failed_child_returns_its_failure_class_without_writing_into_the_checkout() {
    // The child's stream is cut until its requests exhaust (one retry delays the
    // second cut), so it fails on the provider.
    let (fixture, _app, job, provider) =
        run_direct("direct-task-failed", 65, vec!["file-d.txt".into()], vec![5]).await;
    let summary = &tool_result(&provider, "call-task")[0];
    assert_eq!(summary["agent"], "child-one", "{summary}");
    assert_ne!(
        summary["outcome"], "completed",
        "the child failed: {summary}"
    );
    assert!(
        summary["failure_class"].as_str().is_some(),
        "carries the failure class: {summary}"
    );
    assert!(
        summary.get("merged").is_none(),
        "a shared tree reports no merge: {summary}"
    );
    let working = root_checkout(&fixture, &job).await;
    assert!(
        !working.join("file-d.txt").exists(),
        "the failed child left nothing in W"
    );
}

#[tokio::test]
async fn a_join_lands_each_finished_childs_work_in_the_working_checkout_once() {
    // Mode 69 (two `helpers` calls in one turn, each writing a file, then an
    // answer) under a direct-mode job: every finished child writes W directly,
    // so the root's own checkout carries both pieces when the calls return.
    let (fixture, _app, job, _provider) = run_direct(
        "direct-task-await-merge",
        69,
        vec!["new-a.txt".into(), "new-b.txt".into()],
        vec![],
    )
    .await;
    let working = root_checkout(&fixture, &job).await;
    assert_eq!(
        std::fs::read_to_string(working.join("new-a.txt")).unwrap(),
        "native child-a\n"
    );
    assert_eq!(
        std::fs::read_to_string(working.join("new-b.txt")).unwrap(),
        "native child-b\n"
    );
    // Each finished child settled exactly one completion: the join added no
    // second settlement for either piece.
    let mut completions = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap()
        .items;
    completions.sort_by(|a, b| a.result.agent_id().cmp(b.result.agent_id()));
    assert_eq!(completions.len(), 2, "{completions:?}");
    assert_eq!(completions[0].result.agent_id().as_str(), "child-a");
    assert_eq!(completions[1].result.agent_id().as_str(), "child-b");
}

#[tokio::test]
async fn task_is_refused_for_a_job_without_write_paths() {
    let (fixture, _app, job, provider) = run_without_write_paths("direct-task-no-paths").await;
    // Refused: the unadvertised tool is fed back as an unknown-tool error.
    let refusal = tool_result(&provider, "call-task");
    assert!(
        refusal["error"]
            .as_str()
            .is_some_and(|t| t.contains("helpers")),
        "task is refused for a job with no working checkout: {refusal}"
    );
    assert_eq!(
        fixture
            .store
            .list_coding_children(job.job_id(), None, 100)
            .await
            .unwrap()
            .items
            .len(),
        0,
        "no child created"
    );
}

#[tokio::test]
async fn task_is_not_advertised_for_a_job_without_write_paths() {
    let (_, _app, _job, provider) = run_without_write_paths("direct-task-unadvertised").await;
    let advertised: Vec<String> = provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .flat_map(|body| {
            let request: Value = serde_json::from_str(body).unwrap();
            request["tools"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|t| t["function"]["name"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        !advertised.iter().any(|name| name == "helpers"),
        "task is not advertised"
    );
    // The coordinator-only surface advertises none of the retired supervisor
    // tools either: a job with no working checkout has nothing to hand out.
    for name in [
        "delegate",
        "await_children",
        "message_child",
        "cancel_child",
        "coding_status",
        "coding_prepare_candidate",
    ] {
        assert!(
            !advertised.iter().any(|advertised| advertised == name),
            "{name} must stay retired: {advertised:?}"
        );
    }
}

/// Runs a job with no approved write path whose scripted root calls `helpers`
/// (mode 68). Without a working checkout the root gets the coordinator-only
/// toolset, which advertises no `helpers`.
async fn run_without_write_paths(
    label: &str,
) -> (
    Fixture,
    std::sync::Arc<saya_cli::CodingApplication>,
    std::sync::Arc<saya_cli::NativeCodingJob>,
    Provider,
) {
    let provider = Provider::script(false, 68);
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    let application = fixture.application.clone();
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Make a child change".into(),
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    (fixture, application, job, provider)
}
