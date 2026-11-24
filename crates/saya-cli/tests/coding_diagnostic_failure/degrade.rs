//! Dogfood #6: the job's own diagnostics stop being writable mid-run. They
//! carry no authority, so the job records the failure once and keeps its real
//! outcome — while the durable record still fails closed.

use super::{provider, support};
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, CancellationToken, ToolDiagnostic};
use saya_cli::{CodingAuthorization, CodingJobRequest, NativeCodingJob};
use saya_store::SqliteStateStore;
use saya_types::{CodingEventV1, JobId, JobState};
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// Makes the diagnostics writer fail after the first tool call (a full disk).
/// With a `store` it also closes the durable record, so a test can prove that
/// authority still fails closed.
struct StorageFaultSink {
    directory: PathBuf,
    store: Option<SqliteStateStore>,
    observed: AtomicBool,
    closed: AtomicBool,
}

impl StorageFaultSink {
    fn fault(&self) {
        if !self.observed.swap(true, Ordering::AcqRel) {
            std::fs::set_permissions(
                self.directory.join("tool-events.jsonl"),
                std::fs::Permissions::from_mode(0o400),
            )
            .unwrap();
        }
    }
}

#[async_trait]
impl AgentEventSink for StorageFaultSink {
    async fn emit(&self, event: AgentEvent) {
        // The durable record dies on the final answer; authority fails closed.
        if let (Some(store), true) = (
            &self.store,
            matches!(event, AgentEvent::AssistantText { .. }),
        ) && !self.closed.swap(true, Ordering::AcqRel)
        {
            store.close().await;
        }
    }
    async fn observe_tool(&self, _: ToolDiagnostic) {
        self.fault();
    }
}

async fn start(
    label: &str,
    server: &provider::LocalOpenAi,
    close_durable: bool,
) -> (
    support::Fixture,
    Arc<NativeCodingJob>,
    Arc<StorageFaultSink>,
) {
    let fixture = support::Fixture::with_endpoint(label, Some(&server.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
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
    let sink = Arc::new(StorageFaultSink {
        directory,
        store: close_durable.then(|| fixture.store.clone()),
        observed: AtomicBool::new(false),
        closed: AtomicBool::new(false),
    });
    (fixture, job, sink)
}

fn status_call(id: &str) -> String {
    format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":"coding_status","arguments":"{}"}}]},"finish_reason":"tool_calls"}]})
    )
}

/// The notices the durable record holds, decoded: the typed event is how a job
/// states that its diagnostics are incomplete.
fn notices(events: &[CodingEventV1]) -> Vec<Value> {
    events
        .iter()
        .map(|event| serde_json::to_value(event).unwrap())
        .filter(|event| event["event"]["type"] == "diagnostics_incomplete")
        .collect()
}

/// Answers one tool call, then a second — the copy that meets the failing
/// writer.
async fn answer_after_two_tool_calls(server: &mut provider::LocalOpenAi) {
    assert_eq!(server.request().await, Some(1));
    server.respond_sse(&status_call("status-a"));
    assert_eq!(server.request().await, Some(2));
    server.respond_sse(&status_call("status-b"));
}

#[tokio::test]
async fn a_diagnostics_write_failure_degrades_the_job_and_states_diagnostics_incomplete() {
    let mut server = provider::LocalOpenAi::new().await;
    let (fixture, job, sink) = start("diagnostics-storage-degrade", &server, false).await;
    let running = Arc::clone(&job);
    let observing = Arc::clone(&sink);
    let run = tokio::spawn(async move { running.run(observing.as_ref()).await });
    answer_after_two_tool_calls(&mut server).await;
    assert_eq!(server.request().await, Some(3));
    server.respond_sse(provider::ANSWER_RESPONSE);
    let output = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .unwrap()
        .unwrap()
        .expect("a diagnostics write failure carries no authority: the job answers");
    assert_eq!(output.answer, "completed");
    assert!(!output.truncated);
    assert!(sink.observed.load(Ordering::Acquire));
    let status = fixture.application.status(job.job_id()).await.unwrap();
    assert_eq!(status["state"], "completed");
    assert_eq!(
        status["outcome"]["diagnostics_incomplete"], true,
        "{status}"
    );
    let events = fixture
        .store
        .list_coding_events(job.job_id(), None, 100)
        .await
        .unwrap();
    let notices = notices(&events.events);
    assert_eq!(
        notices.len(),
        1,
        "one notice, never a retry storm: {:?}",
        events.events
    );
    assert_eq!(notices[0]["event"]["fault"], "storage");
    // Once the writer failed, the job stopped writing diagnostics: its own end
    // record was never attempted.
    let observations =
        std::fs::read_to_string(sink.directory.join("observations.jsonl")).unwrap_or_default();
    assert!(
        !observations.contains("\"phase\":\"finished\""),
        "the job stopped writing diagnostics: {observations}"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn a_real_failure_keeps_its_class_when_the_diagnostics_also_fail() {
    let mut server = provider::LocalOpenAi::new().await;
    let (fixture, job, sink) = start("diagnostics-storage-keeps-cause", &server, false).await;
    let running = Arc::clone(&job);
    let observing = Arc::clone(&sink);
    let run = tokio::spawn(async move { running.run(observing.as_ref()).await });
    assert_eq!(server.request().await, Some(1));
    server.respond_sse(&status_call("status-a"));
    assert_eq!(server.request().await, Some(2));
    server.respond_sse(&status_call("status-b"));
    assert_eq!(server.request().await, Some(3));
    server.respond_status(500);
    let error = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("provider_error"), "{error}");
    let status = fixture.application.status(job.job_id()).await.unwrap();
    assert_eq!(status["state"], "failed");
    assert_eq!(
        status["failure"]["event"]["failure"]["class"],
        "provider_error"
    );
    assert_eq!(
        status["failure"]["event"]["secondary"],
        Value::Null,
        "degraded diagnostics are not a second failure: {status}"
    );
    let events = fixture
        .store
        .list_coding_events(job.job_id(), None, 100)
        .await
        .unwrap();
    assert_eq!(notices(&events.events).len(), 1, "{:?}", events.events);
    server.shutdown().await;
}

#[tokio::test]
async fn a_durable_record_that_cannot_be_written_still_fails_the_job() {
    let mut server = provider::LocalOpenAi::new().await;
    let (fixture, job, sink) = start("diagnostics-storage-durable", &server, true).await;
    let running = Arc::clone(&job);
    let observing = Arc::clone(&sink);
    let run = tokio::spawn(async move { running.run(observing.as_ref()).await });
    assert_eq!(server.request().await, Some(1));
    server.respond_sse(&status_call("status-a"));
    assert_eq!(server.request().await, Some(2));
    server.respond_sse(&status_call("status-b"));
    assert_eq!(server.request().await, Some(3));
    server.respond_sse(provider::ANSWER_RESPONSE);
    // The sink had already made the diagnostics writer fail and closes the
    // durable record when the answer arrives.
    let error = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    let text = error.to_string();
    assert!(
        text.contains("terminal state could not be confirmed")
            && text.contains("MandatoryPersistence"),
        "the durable record must still fail closed: {text}"
    );
    assert!(
        !text.contains("DiagnosticStorage"),
        "a degraded diagnostics fault must never become the job's failure: {text}"
    );
    let reopened = SqliteStateStore::new(fixture.store.database_path());
    let record = reopened
        .get_coding_job(job.job_id())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        record.state,
        JobState::Completed,
        "the durable record is the authority: a run that cannot write it never completes"
    );
    reopened.close().await;
    server.shutdown().await;
}
