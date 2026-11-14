//! Failure identity survives an already-consumed join and secondary diagnostic I/O.
#![cfg(unix)]
#[path = "../../saya-harness/tests/coding_runtime_contracts/budgeted_child_scheduler/openai_server.rs"]
#[allow(dead_code)]
// Reuse the bounded scripted HTTP transport without adding a second implementation.
mod provider;
#[path = "coding_application/support.rs"]
#[allow(dead_code)] // Shared public-owner fixture; this suite uses its endpoint setup.
mod support;
use provider::{ANSWER_RESPONSE, LocalOpenAi};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest, CodingTask, NativeCodingJob};
use saya_types::{AgentId, JobId, JobState};
use std::{sync::Arc, time::Duration};
use support::Fixture;

async fn create(label: &str, server: &LocalOpenAi) -> (Fixture, Arc<NativeCodingJob>) {
    let fixture = Fixture::with_endpoint(label, Some(&server.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "inspect assigned work".into(),
        vec!["child.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    (fixture, job)
}

#[tokio::test]
async fn consumed_child_join_preserves_safe_cause_and_identity_through_parent_finalization() {
    let mut server = LocalOpenAi::new().await;
    let (fixture, job) = create("consumed-child-failure", &server).await;
    job.delegate(vec![CodingTask {
        agent_id: AgentId::parse("child-failure").unwrap(),
        subagent_type: "general".into(),
        task: "inspect assigned work".into(),
        write_paths: vec!["child.txt".into()],
        dependencies: vec![],
    }])
    .await
    .unwrap();
    assert_eq!(server.request().await, Some(1));
    server.respond_status(500);
    let report = tokio::time::timeout(Duration::from_secs(10), job.join())
        .await
        .unwrap()
        .unwrap();
    assert!(report.outcomes[0].result.is_err());
    let running = Arc::clone(&job);
    let run = tokio::spawn(async move { running.run(&NoopEventSink).await });
    assert_eq!(server.request().await, Some(2));
    server.respond_sse(ANSWER_RESPONSE);
    // Direct mode deliberately does not latch a child's own failure class onto
    // the job: the root consumed the failed join and answered, so the run
    // succeeds and the job carries no failure at all (`failure.rs` skips the
    // child class for Direct). The claim the test is about is unchanged and is
    // asserted where the child's failure is owned: the already-consumed join
    // report and the durable child record keep the cause's identity through
    // parent finalization, and the original repository stays untouched.
    let outcome = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .unwrap()
        .unwrap()
        .expect("the root answers after consuming the failed child join");
    assert!(!outcome.truncated);
    assert_eq!(
        job.failure(),
        None,
        "a consumed child failure is not latched as the job's failure"
    );
    let status = fixture.application.status(job.job_id()).await.unwrap();
    assert_eq!(status["state"], "completed");
    assert_eq!(status["failure"], serde_json::Value::Null);
    let children = status["children"].as_array().unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["agent_id"], "child-failure");
    assert_eq!(children[0]["state"], "failed");
    assert_eq!(
        children[0]["invocation_id"],
        serde_json::to_value(&report.outcomes[0].invocation_id).unwrap(),
        "the failed child keeps the identity its consumed join reported"
    );
    assert!(!fixture.repository.join("child.txt").exists());
    server.shutdown().await;
}

#[tokio::test]
async fn secondary_finished_diagnostic_failure_cannot_skip_durable_primary_failure() {
    let mut server = LocalOpenAi::new().await;
    let (fixture, job) = create("secondary-diagnostic-failure", &server).await;
    let running = Arc::clone(&job);
    let run = tokio::spawn(async move { running.run(&NoopEventSink).await });
    assert_eq!(server.request().await, Some(1));
    let directory = fixture.root.join("coding").join(job.job_id().as_str());
    let target = fixture.root.join("unrelated-preserved");
    std::fs::write(&target, "PRIVATE_PAYLOAD_SENTINEL").unwrap();
    std::fs::rename(
        directory.join("observations.jsonl"),
        directory.join("preserved-observations.jsonl"),
    )
    .unwrap();
    std::os::unix::fs::symlink(&target, directory.join("observations.jsonl")).unwrap();
    server.respond_status(500);
    let error = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        error.to_string().contains("provider_error"),
        "secondary diagnostic replaced primary: {error}"
    );
    let record = fixture
        .store
        .get_coding_job(job.job_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        record.state,
        JobState::Failed,
        "diagnostics cannot leave a failed run active"
    );
    let status = fixture.application.status(job.job_id()).await.unwrap();
    assert_eq!(
        status["failure"]["event"]["failure"]["class"],
        "provider_error"
    );
    assert_eq!(
        status["failure"]["event"]["secondary"],
        "diagnostic_storage"
    );
    assert_eq!(
        status["failure"]["event"]["diagnostic_fault"],
        "unsafe_path"
    );
    assert!(!status.to_string().contains("PRIVATE_PAYLOAD_SENTINEL"));
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "PRIVATE_PAYLOAD_SENTINEL"
    );
    let exported = fixture
        .application
        .export(job.job_id(), None, None)
        .await
        .expect("diagnostic fault cannot hide durable failure export");
    assert_eq!(
        exported["status"]["failure"]["event"]["failure"]["class"],
        "provider_error"
    );
    assert_eq!(
        exported["artifacts"]["diagnostics"]["history"]["state"],
        "unknown"
    );
    assert_eq!(exported["artifacts"]["diagnostics"]["fault"], "unsafe_path");
    assert_eq!(exported["artifacts"]["complete"], false);
    assert!(!exported.to_string().contains("PRIVATE_PAYLOAD_SENTINEL"));
    server.shutdown().await;
}

#[tokio::test]
async fn authoritative_terminal_failure_is_reported_as_unconfirmed_and_keeps_unknown_charges() {
    let mut server = LocalOpenAi::new().await;
    let (fixture, job) = create("terminal-state-unconfirmed", &server).await;
    let running = Arc::clone(&job);
    let run = tokio::spawn(async move { running.run(&NoopEventSink).await });
    assert_eq!(server.request().await, Some(1));
    fixture.store.close().await;
    server.respond_status(500);
    let error = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("terminal state could not be confirmed"),
        "missing terminal persistence must be explicit: {error}"
    );
    assert!(!error.to_string().contains("recorded as failed"));
    let reopened = saya_store::SqliteStateStore::new(fixture.store.database_path());
    let record = reopened
        .get_coding_job(job.job_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        record.state,
        JobState::Running,
        "the owner must not invent a durable failure"
    );
    assert!(
        reopened
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations
            > 0
    );
    reopened.close().await;
    server.shutdown().await;
}
