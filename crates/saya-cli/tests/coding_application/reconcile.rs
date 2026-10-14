use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, dispatch_coding,
};
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetReservationV1, CodingEventKind, CodingEventV1, JobId,
};
use std::num::NonZeroU64;

/// A job that reached `Running` and then crashed in the middle of an effect:
/// one open (never settled) reservation in its stream.
async fn crashed_job(fixture: &Fixture, job: &str) -> JobId {
    let request = CodingJobRequest::new(
        JobId::parse(job).unwrap(),
        "inspect only".into(),
        vec![],
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let runtime = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let id = runtime.job_id().clone();
    drop(runtime);
    let status = fixture.store.coding_recovery_status(&id).await.unwrap();
    // A running job's writes are fenced by its runtime authority.
    let authority = status
        .authority
        .expect("a created job has runtime authority");
    let next = status.last_sequence + 1;
    let sequence = NonZeroU64::new(next).unwrap();
    let reservation = BudgetReservationV1::new(
        id.clone(),
        sequence,
        None,
        None,
        vec![
            BudgetCharge::new(
                BudgetDimension::LogicalRequests,
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
                id.clone(),
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
    id
}

fn reconcile(job: &JobId, digest: Option<&str>, approve: bool) -> CodingCommand {
    CodingCommand::Reconcile {
        id: job.as_str().into(),
        plan_sha256: digest.map(str::to_owned),
        approve,
    }
}

async fn unresolved(fixture: &Fixture, job: &JobId) -> u64 {
    fixture
        .store
        .coding_recovery_status(job)
        .await
        .unwrap()
        .unresolved_reservations
}

#[tokio::test]
async fn reconcile_lists_then_records_exactly_the_listed_effects_and_resume_follows() {
    let fixture = Fixture::new("reconcile-flow").await;
    let job = crashed_job(&fixture, "job-reconcile").await;
    assert_eq!(unresolved(&fixture, &job).await, 1);

    let listed = dispatch_coding(&fixture.application, reconcile(&job, None, false))
        .await
        .unwrap();
    assert_eq!(listed["changes_nothing"], true);
    assert_eq!(listed["effects"].as_array().unwrap().len(), 1);
    assert_eq!(listed["effects"][0]["settled_uncertain"], false);
    let digest = listed["plan_sha256"].as_str().unwrap().to_owned();
    assert!(
        listed["next_action"].as_str().unwrap().contains(&digest),
        "the next action carries the exact digest: {listed}"
    );
    assert_eq!(
        unresolved(&fixture, &job).await,
        1,
        "listing writes nothing"
    );

    // Without --approve nothing is recorded; with it, exactly the listed plan.
    assert!(matches!(
        dispatch_coding(&fixture.application, reconcile(&job, Some(&digest), false)).await,
        Err(CodingError::ApprovalRequired)
    ));
    assert_eq!(unresolved(&fixture, &job).await, 1);
    let done = dispatch_coding(&fixture.application, reconcile(&job, Some(&digest), true))
        .await
        .unwrap();
    assert_eq!(done["refunded"], false);
    assert_eq!(done["reconciled"].as_array().unwrap().len(), 1);
    assert_eq!(unresolved(&fixture, &job).await, 0);

    // Resume is now reachable, and still needs its own fresh approval.
    let consent =
        CodingAuthorization::scoped(&fixture.application.propose_resume(&job).await.unwrap())
            .unwrap();
    fixture
        .application
        .resume(&job, &consent, CancellationToken::new())
        .await
        .expect("a reconciled job resumes");
}

#[tokio::test]
async fn a_stale_digest_and_bad_arguments_are_refused_and_write_nothing() {
    let fixture = Fixture::new("reconcile-stale").await;
    let job = crashed_job(&fixture, "job-reconcile-stale").await;
    let stale = "0".repeat(64);
    assert!(matches!(
        dispatch_coding(&fixture.application, reconcile(&job, Some(&stale), true)).await,
        Err(CodingError::ReconciliationStale)
    ));
    assert!(matches!(
        dispatch_coding(&fixture.application, reconcile(&job, None, true)).await,
        Err(CodingError::InvalidRequest)
    ));
    assert!(matches!(
        dispatch_coding(
            &fixture.application,
            reconcile(&job, Some("not-a-digest"), true)
        )
        .await,
        Err(CodingError::Owner(_))
    ));
    assert_eq!(
        unresolved(&fixture, &job).await,
        1,
        "every refusal wrote nothing"
    );
    // Positive control: the current digest reconciles.
    let listed = dispatch_coding(&fixture.application, reconcile(&job, None, false))
        .await
        .unwrap();
    let digest = listed["plan_sha256"].as_str().unwrap();
    dispatch_coding(&fixture.application, reconcile(&job, Some(digest), true))
        .await
        .unwrap();
    assert_eq!(unresolved(&fixture, &job).await, 0);
}

#[tokio::test]
async fn an_unknown_job_is_not_owned() {
    let fixture = Fixture::new("reconcile-unknown").await;
    let unknown = JobId::parse("job-never-created").unwrap();
    assert!(matches!(
        dispatch_coding(&fixture.application, reconcile(&unknown, None, false)).await,
        Err(CodingError::NotOwned)
    ));
}
