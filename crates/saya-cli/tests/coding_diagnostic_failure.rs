//! A later reporting fault must not replace the first diagnosed storage fault.
#![cfg(unix)]
#[path = "../../saya-harness/tests/coding_runtime_contracts/budgeted_child_scheduler/openai_server.rs"]
#[allow(dead_code)] // Shared bounded HTTP transport fixture.
mod provider;
#[path = "coding_application/support.rs"]
#[allow(dead_code)] // Shared real application fixture.
mod support;
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, CancellationToken, ToolDiagnostic};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
struct FaultSink {
    directory: PathBuf,
    observed: AtomicBool,
}
#[async_trait]
impl AgentEventSink for FaultSink {
    async fn emit(&self, _: AgentEvent) {}
    async fn observe_tool(&self, _: ToolDiagnostic) {
        if !self.observed.swap(true, Ordering::AcqRel) {
            // NativeSink has already observed UnsafePath for the tool journal.
            std::fs::rename(
                self.directory.join("tool-events.jsonl"),
                self.directory.join("preserved-first-fault-symlink"),
            )
            .unwrap();
            std::fs::write(
                self.directory.join("diagnostics-summary.json"),
                b"malformed",
            )
            .unwrap();
        }
    }
}
#[tokio::test]
async fn first_diagnostic_fault_survives_later_finalization_recovery_fault() {
    let mut server = provider::LocalOpenAi::new().await;
    let fixture =
        support::Fixture::with_endpoint("first-diagnostic-fault", Some(&server.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("first-diagnostic-fault").unwrap(),
        "read coding status".into(),
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let directory = fixture.root.join("coding").join(job.job_id().as_str());
    let sink = Arc::new(FaultSink {
        directory: directory.clone(),
        observed: AtomicBool::new(false),
    });
    let running = Arc::clone(&job);
    let observing = Arc::clone(&sink);
    let run = tokio::spawn(async move { running.run(observing.as_ref()).await });
    assert_eq!(server.request().await, Some(1));
    let unrelated = fixture.root.join("untouched");
    std::fs::write(&unrelated, b"PRIVATE_DIAGNOSTIC_SENTINEL").unwrap();
    std::os::unix::fs::symlink(&unrelated, directory.join("tool-events.jsonl")).unwrap();
    let chunk = json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"status","type":"function","function":{"name":"coding_status","arguments":"{}"}}]},"finish_reason":"tool_calls"}]});
    server.respond_sse(&format!("data: {chunk}\n\ndata: [DONE]\n\n"));
    let error = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("diagnostic_storage"));
    assert!(sink.observed.load(Ordering::Acquire));
    let status = fixture.application.status(job.job_id()).await.unwrap();
    assert_eq!(
        status["failure"]["event"]["diagnostic_fault"],
        "unsafe_path"
    );
    assert_eq!(status["diagnostic_history"]["fault"], "recovery");
    assert_eq!(
        std::fs::read(&unrelated).unwrap(),
        b"PRIVATE_DIAGNOSTIC_SENTINEL"
    );
    assert!(!status.to_string().contains("PRIVATE_DIAGNOSTIC_SENTINEL"));
    server.shutdown().await;
}
