use super::support::Fixture;
use async_trait::async_trait;
use saya_agent::{ApprovalDecider, CancellationToken, ToolDefinition};
use saya_cli::{CodingAuthorization, CodingError, CodingJobRequest};
use saya_store::CodingApprovalRequestRecord;
use saya_types::{
    ApprovalDisposition, ApprovalRequestContext, ApprovalRequestV1, ApprovalScope, CodingEventKind,
    CodingEventV1, JobApprovalAction, JobId,
};
use std::num::NonZeroU64;

struct Deny;
#[async_trait]
impl ApprovalDecider for Deny {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        false
    }
}

async fn created(fixture: &Fixture, job: &str) -> JobId {
    let request = CodingJobRequest::new(
        JobId::parse(job).unwrap(),
        "inspect only".into(),
        vec![],
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap()
        .job_id()
        .clone()
}

async fn audit(fixture: &Fixture, job: &JobId) -> Vec<CodingApprovalRequestRecord> {
    fixture
        .store
        .list_coding_approval_requests(job, None, 100)
        .await
        .unwrap()
        .items
}

fn action(record: &CodingApprovalRequestRecord) -> JobApprovalAction {
    match record.request.scope() {
        ApprovalScope::JobOperation { action, .. } => *action,
        other => panic!("not a job-level scope: {other:?}"),
    }
}

fn closed_as(record: &CodingApprovalRequestRecord) -> &ApprovalDisposition {
    &record
        .closure
        .as_ref()
        .expect("the request is closed")
        .disposition
}

async fn resume_ok(fixture: &Fixture, job: &JobId) {
    let consent =
        CodingAuthorization::scoped(&fixture.application.propose_resume(job).await.unwrap())
            .unwrap();
    fixture
        .application
        .resume(job, &consent, CancellationToken::new())
        .await
        .expect("resume");
}

async fn pending(fixture: &Fixture, job: &JobId) -> u64 {
    fixture
        .store
        .coding_recovery_status(job)
        .await
        .unwrap()
        .pending_approvals
}

/// A request left open by a process that died while waiting for a decision.
async fn abandon_a_wait(fixture: &Fixture, job: &JobId) {
    let status = fixture.store.coding_recovery_status(job).await.unwrap();
    let sequence = NonZeroU64::new(status.last_sequence + 1).unwrap();
    let digest = saya_types::Sha256Digest::parse(&"c".repeat(64)).unwrap();
    let request = ApprovalRequestV1::new_for_coordinator(
        job.clone(),
        sequence,
        ApprovalRequestContext::new(digest.clone(), 1, 1, 2).unwrap(),
        ApprovalScope::job_operation(JobApprovalAction::Resume, digest),
    )
    .unwrap();
    fixture
        .store
        .record_coding_approval_request(
            CodingEventV1::new(
                sequence,
                job.clone(),
                None,
                None,
                CodingEventKind::ApprovalRequested {
                    request: Box::new(request),
                },
            )
            .unwrap(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_created_job_records_its_start_approval_as_granted() {
    let fixture = Fixture::new("audit-create").await;
    let job = created(&fixture, "job-audit-create").await;
    let records = audit(&fixture, &job).await;
    assert_eq!(records.len(), 1);
    assert_eq!(action(&records[0]), JobApprovalAction::Start);
    assert_eq!(closed_as(&records[0]), &ApprovalDisposition::Granted);
    assert_eq!(pending(&fixture, &job).await, 0);
}

#[tokio::test]
async fn resume_audits_the_ask_and_the_answer_and_a_denial_is_never_a_grant() {
    let fixture = Fixture::new("audit-resume").await;
    let job = created(&fixture, "job-audit-resume").await;

    let denied = fixture
        .application
        .resume(&job, &Deny, CancellationToken::new())
        .await;
    assert!(matches!(denied, Err(CodingError::ApprovalRequired)));
    let records = audit(&fixture, &job).await;
    assert_eq!(records.len(), 2);
    assert_eq!(action(&records[1]), JobApprovalAction::Resume);
    assert_eq!(closed_as(&records[1]), &ApprovalDisposition::Denied);
    assert_eq!(
        pending(&fixture, &job).await,
        0,
        "a denial leaves nothing open"
    );

    // Positive control: the same job resumes with real consent, audited as granted.
    resume_ok(&fixture, &job).await;
    let records = audit(&fixture, &job).await;
    assert_eq!(records.len(), 3);
    assert_eq!(closed_as(&records[2]), &ApprovalDisposition::Granted);
}

#[tokio::test]
async fn an_abandoned_wait_is_cancelled_at_the_next_resume() {
    let fixture = Fixture::new("audit-stale").await;
    let job = created(&fixture, "job-audit-stale").await;
    abandon_a_wait(&fixture, &job).await;
    assert_eq!(pending(&fixture, &job).await, 1);
    let status = fixture.store.coding_recovery_status(&job).await.unwrap();
    assert!(!status.clear_for_resume(), "an open request blocks resume");

    resume_ok(&fixture, &job).await;
    let records = audit(&fixture, &job).await;
    let dispositions: Vec<&ApprovalDisposition> = records.iter().map(closed_as).collect();
    assert_eq!(
        dispositions,
        vec![
            &ApprovalDisposition::Granted,   // the create-time audit
            &ApprovalDisposition::Cancelled, // the abandoned wait
            &ApprovalDisposition::Granted,   // this resume
        ]
    );
    assert_eq!(pending(&fixture, &job).await, 0);
}

#[tokio::test]
async fn an_open_request_is_not_cancelled_while_another_blocker_remains() {
    let fixture = Fixture::new("audit-guard").await;
    let job = created(&fixture, "job-audit-guard").await;
    // A second blocker: an unresolved reservation under the job's authority.
    let status = fixture.store.coding_recovery_status(&job).await.unwrap();
    let authority = status.authority.clone().expect("runtime authority");
    let sequence = NonZeroU64::new(status.last_sequence + 1).unwrap();
    let reservation = saya_types::BudgetReservationV1::new(
        job.clone(),
        sequence,
        None,
        None,
        vec![
            saya_types::BudgetCharge::new(
                saya_types::BudgetDimension::LogicalRequests,
                None,
                None,
                NonZeroU64::new(1).unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    fixture
        .store
        .reserve_coding_budget_fenced(
            CodingEventV1::new(
                sequence,
                job.clone(),
                None,
                None,
                CodingEventKind::BudgetReserved {
                    reservation: Box::new(reservation),
                },
            )
            .unwrap(),
            Some(&authority),
        )
        .await
        .unwrap();
    abandon_a_wait(&fixture, &job).await;

    let consent =
        CodingAuthorization::scoped(&fixture.application.propose_resume(&job).await.unwrap())
            .unwrap();
    let refused = fixture
        .application
        .resume(&job, &consent, CancellationToken::new())
        .await;
    assert!(matches!(refused, Err(CodingError::NotOwned)));
    // The abandoned request stays open: the guard saw another blocker and
    // touched nothing. This resume's own ask was answered, so it is closed.
    assert_eq!(pending(&fixture, &job).await, 1);
    let records = audit(&fixture, &job).await;
    assert_eq!(records.len(), 3);
    assert!(
        records[1].closure.is_none(),
        "the abandoned wait is untouched"
    );
    assert_eq!(closed_as(&records[2]), &ApprovalDisposition::Granted);
}
