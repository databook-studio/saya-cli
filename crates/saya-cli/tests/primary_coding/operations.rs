use super::support::Fixture;
use saya_agent::{AllowReadOnlyApproval, CancellationToken};
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, dispatch_coding,
};
use saya_types::{JobId, ManagedCheckoutId};

#[tokio::test]
async fn command_and_application_refuse_unapproved_job_without_durable_effects() {
    let fixture = Fixture::new("approval-parity").await;
    let job = JobId::parse("job-denied").unwrap();
    let request = CodingJobRequest::new(
        job.clone(),
        "bounded task".into(),
        vec!["tracked.txt".into()],
        vec![],
    )
    .unwrap();
    assert!(matches!(
        fixture
            .application
            .create(request, &AllowReadOnlyApproval, CancellationToken::new())
            .await,
        Err(CodingError::ApprovalRequired)
    ));
    assert!(matches!(
        dispatch_coding(
            &fixture.application,
            CodingCommand::Start {
                id: job.as_str().into(),
                task: "bounded task".into(),
                write_paths: vec!["tracked.txt".into()],
                programs: vec![],
                approve: false
            }
        )
        .await,
        Err(CodingError::ApprovalRequired)
    ));
    assert!(fixture.store.get_coding_job(&job).await.unwrap().is_none());
    assert_eq!(
        std::fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
}
#[tokio::test]
async fn managed_worktree_operation_refuses_foreign_removal() {
    let fixture = Fixture::new("foreign-removal").await;
    let request = CodingJobRequest::new(
        JobId::parse("job-owned").unwrap(),
        "inspect only".into(),
        vec![],
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
    assert!(
        job.remove_checkout(&ManagedCheckoutId::parse("foreign").unwrap())
            .await
            .is_err()
    );
    assert!(fixture.repository.join(".git").exists());
    assert_eq!(
        std::fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
}

#[tokio::test]
async fn stale_snapshot_proposal_refuses_before_creating_job_and_fresh_retry_succeeds() {
    let fixture = Fixture::new("proposal-drift").await;
    let request = CodingJobRequest::new(
        JobId::parse("job-proposal").unwrap(),
        "inspect snapshot".into(),
        vec![],
        vec![],
    )
    .unwrap();
    let stale =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    std::fs::write(
        fixture.repository.join("tracked.txt"),
        b"user changed before approval\n",
    )
    .unwrap();
    assert!(matches!(
        fixture
            .application
            .create(request.clone(), &stale, CancellationToken::new())
            .await,
        Err(CodingError::ApprovalRequired)
    ));
    assert!(
        fixture
            .store
            .get_coding_job(request.job_id())
            .await
            .unwrap()
            .is_none()
    );
    let fresh =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    fixture
        .application
        .create(request, &fresh, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"user changed before approval\n"
    );
}
#[tokio::test]
async fn list_shows_only_jobs_of_this_repository() {
    let fixture = Fixture::new("list-identity").await;
    let request = CodingJobRequest::new(
        JobId::parse("job-owned").unwrap(),
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
        .unwrap();
    fixture
        .store
        .create_coding_job(saya_store::NewCodingJob {
            job_id: JobId::parse("job-foreign").unwrap(),
            repository_id: saya_types::RepositoryId::parse("foreign-repository").unwrap(),
            snapshot_id: saya_types::SnapshotId::parse("foreign-snapshot").unwrap(),
        })
        .await
        .unwrap();
    let listed = dispatch_coding(&fixture.application, CodingCommand::List { after: None })
        .await
        .unwrap();
    let jobs = listed["jobs"].as_array().unwrap();
    assert!(jobs.iter().any(|job| job["job_id"] == "job-owned"));
    assert!(!jobs.iter().any(|job| job["job_id"] == "job-foreign"));
}
#[tokio::test]
async fn retain_routes_to_the_shared_operation() {
    let fixture = Fixture::new("retain-route").await;
    let request = CodingJobRequest::new(
        JobId::parse("job-retain").unwrap(),
        "inspect only".into(),
        vec![],
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
    let candidate = ManagedCheckoutId::parse("retain-candidate").unwrap();
    job.create_checkout(&candidate).await.unwrap();
    let retained = dispatch_coding(
        &fixture.application,
        CodingCommand::RetainCheckout {
            id: "job-retain".into(),
            checkout: "retain-candidate".into(),
            approve: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(retained["retained"], "retain-candidate");
    let worktrees = fixture.application.worktrees(job.job_id()).await.unwrap();
    let checkouts = worktrees["checkouts"].as_array().unwrap();
    let record = checkouts
        .iter()
        .find(|checkout| checkout["checkout_id"] == "retain-candidate")
        .unwrap();
    assert_eq!(record["lifecycle"], "active");
}
