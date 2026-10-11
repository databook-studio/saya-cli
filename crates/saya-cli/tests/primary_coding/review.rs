use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{CodingAuthorization, CodingError, CodingJobRequest};
use saya_types::{JobId, ManagedCheckoutId};
#[tokio::test]
async fn candidate_edit_invalidates_the_actual_review_before_apply() {
    let fixture = Fixture::new("review-stale").await;
    let request = CodingJobRequest::new(
        JobId::parse("job-review").unwrap(),
        "review changes".into(),
        vec!["tracked.txt".into()],
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
    let candidate = ManagedCheckoutId::parse("review-candidate").unwrap();
    job.create_checkout(&candidate).await.unwrap();
    let review = job.review_existing(&candidate).await.unwrap();
    let record = fixture
        .store
        .get_managed_checkout(job.job_id(), &candidate)
        .await
        .unwrap()
        .unwrap();
    std::fs::write(
        std::path::Path::new(record.ownership.managed_path().as_str()).join("tracked.txt"),
        b"changed after review\n",
    )
    .unwrap();
    assert!(matches!(
        job.apply_review(&review, true).await,
        Err(CodingError::StaleReview)
    ));
    assert_eq!(
        std::fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
}
