use super::support::Fixture;
use async_trait::async_trait;
use saya_agent::{ApprovalDecider, CancellationToken, ToolDefinition};
use saya_cli::{CodingApplication, CodingAuthorization, CodingError, CodingJobRequest};
use saya_store::{CodingApprovalRequestRecord, SqliteStateStore};
use saya_types::{ApprovalDisposition, ApprovalScope, JobApprovalAction, JobId};
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, oneshot};

struct GateApproval {
    entered: mpsc::UnboundedSender<Value>,
    answer: Mutex<Option<oneshot::Receiver<bool>>>,
}

#[async_trait]
impl ApprovalDecider for GateApproval {
    async fn approve(&self, _: &ToolDefinition, proposed: &Value) -> bool {
        assert!(self.entered.send(proposed.clone()).is_ok());
        let answer = { self.answer.lock().await.take().expect("one resume ask") };
        answer.await.expect("test releases the first resume ask")
    }
}

struct CountingApproval(AtomicUsize);

#[async_trait]
impl ApprovalDecider for CountingApproval {
    async fn approve(&self, _: &ToolDefinition, _: &Value) -> bool {
        self.0.fetch_add(1, Ordering::SeqCst);
        true
    }
}

async fn request_records(
    store: &SqliteStateStore,
    job: &JobId,
) -> Vec<CodingApprovalRequestRecord> {
    store
        .list_coding_approval_requests(job, None, 100)
        .await
        .expect("approval audit is readable")
        .items
}

fn action(record: &CodingApprovalRequestRecord) -> JobApprovalAction {
    match record.request.scope() {
        ApprovalScope::JobOperation { action, .. } => *action,
        other => panic!("not a job-level approval: {other:?}"),
    }
}

#[tokio::test]
async fn concurrent_resume_preserves_live_request_across_store_handles() {
    let fixture = Fixture::new("audit-resume-concurrency").await;
    let request = CodingJobRequest::new(
        JobId::parse("job-audit-resume-concurrency").unwrap(),
        "inspect only".into(),
        vec![],
    )
    .unwrap();
    let start_approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &start_approval, CancellationToken::new())
        .await
        .expect("start job")
        .job_id()
        .clone();

    // A distinct pool against the same SQLite file proves the owner decision
    // is coordinated by durable storage rather than one application's memory.
    let second_store = SqliteStateStore::new(fixture.root.join("state.sqlite3"));
    let second_application = CodingApplication::open(
        fixture.runtime.clone(),
        second_store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .expect("second application handle opens on the shared store");

    let (entered_tx, mut entered_rx) = mpsc::unbounded_channel();
    let (answer_tx, answer_rx) = oneshot::channel();
    let first_decider = Arc::new(GateApproval {
        entered: entered_tx,
        answer: Mutex::new(Some(answer_rx)),
    });
    let first_application = Arc::clone(&fixture.application);
    let first_job = job.clone();
    let first = tokio::spawn(async move {
        first_application
            .resume(&first_job, first_decider.as_ref(), CancellationToken::new())
            .await
    });

    let proposal = match tokio::time::timeout(Duration::from_secs(5), entered_rx.recv()).await {
        Ok(Some(proposal)) => proposal,
        _ => {
            let _ = answer_tx.send(true);
            let _ = first.await;
            panic!("first resume did not reach its approval decision");
        }
    };
    assert_eq!(proposal["operation"], "resume");
    let while_waiting = request_records(&fixture.store, &job).await;
    assert_eq!(while_waiting.len(), 2, "start and one resume request");
    assert_eq!(action(&while_waiting[1]), JobApprovalAction::Resume);
    assert!(while_waiting[1].closure.is_none());

    let second_decider = CountingApproval(AtomicUsize::new(0));
    let second_result = match tokio::time::timeout(
        Duration::from_secs(5),
        second_application.resume(&job, &second_decider, CancellationToken::new()),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            let _ = answer_tx.send(true);
            let _ = first.await;
            panic!("second resume did not return while the first ask was pending");
        }
    };
    let second_decider_calls = second_decider.0.load(Ordering::SeqCst);
    let after_second = request_records(&fixture.store, &job).await;

    assert!(answer_tx.send(true).is_ok());
    let first_result = tokio::time::timeout(Duration::from_secs(5), first)
        .await
        .expect("first resume settles")
        .expect("first resume task joins");
    let first_succeeded = first_result.is_ok();
    drop(first_result);
    let second_was_refused = matches!(second_result, Err(CodingError::Busy));
    drop(second_result);
    drop(second_application);
    second_store.close().await;

    assert!(
        second_was_refused,
        "a second resume must report busy while the first approval is live"
    );
    assert_eq!(
        second_decider_calls, 0,
        "a live request is not replaced by a new ask"
    );
    assert_eq!(after_second.len(), 2, "the second call records no request");
    assert!(
        after_second[1].closure.is_none(),
        "the first request stays open"
    );
    assert!(first_succeeded, "the live approval can still complete");

    let records = request_records(&fixture.store, &job).await;
    assert_eq!(records.len(), 2);
    assert_eq!(action(&records[0]), JobApprovalAction::Start);
    assert_eq!(
        &records[0].closure.as_ref().unwrap().disposition,
        &ApprovalDisposition::Granted
    );
    assert_eq!(action(&records[1]), JobApprovalAction::Resume);
    assert_eq!(
        &records[1].closure.as_ref().unwrap().disposition,
        &ApprovalDisposition::Granted
    );
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(&job)
            .await
            .unwrap()
            .pending_approvals,
        0
    );
}
