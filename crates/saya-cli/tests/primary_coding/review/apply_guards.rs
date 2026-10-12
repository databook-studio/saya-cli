//! Review/apply guards: a candidate edit or a user edit after the review
//! was captured makes apply refuse, with both edits intact.

use super::super::support::Fixture;
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
    assert_eq!(review.preview()["checks_configured"], false);
    assert_eq!(review.preview()["diagnostics_configured"], false);
    assert_eq!(
        review.preview()["diagnostic_delta"],
        serde_json::json!({"available":false,"reason":"comparable_baseline_not_minted"})
    );
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

#[tokio::test]
async fn review_apply_after_user_drift_refuses_and_keeps_both_edits() {
    let output = r#"{"reason":"compiler-message","message":{"message":"drift fixture","code":{"code":"unused_variables"},"level":"warning","spans":[{"file_name":"tracked.txt","line_start":1,"line_end":1,"column_start":1,"column_end":2,"is_primary":true}]}}"#;
    let fixture = Fixture::with_diagnostic_output("review-user-drift", output).await;
    let request = CodingJobRequest::new(
        JobId::parse("job-review-user-drift").unwrap(),
        "review user drift".into(),
        vec!["tracked.txt".into()],
        vec!["printf".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let candidate = ManagedCheckoutId::parse("review-user-drift-candidate").unwrap();
    job.create_checkout(&candidate).await.unwrap();
    let record = fixture
        .store
        .get_managed_checkout(job.job_id(), &candidate)
        .await
        .unwrap()
        .unwrap();
    let candidate_root = std::path::Path::new(record.ownership.managed_path().as_str());
    std::fs::write(candidate_root.join("tracked.txt"), b"candidate change\n").unwrap();
    let review = job.review_existing(&candidate).await.unwrap();
    assert_eq!(review.preview()["checks"][0]["successful"], true);
    std::fs::write(fixture.repository.join("tracked.txt"), b"user edit\n").unwrap();
    let Err(refusal) = job.apply_review(&review, true).await else {
        panic!("apply must be refused after the user edited the repository");
    };
    let CodingError::Owner(source) = &refusal else {
        panic!("unexpected refusal variant: {refusal}");
    };
    assert_eq!(
        source.to_string(),
        "the original working tree no longer matches the captured baseline"
    );
    assert_eq!(
        std::fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"user edit\n"
    );
    assert_eq!(
        std::fs::read(candidate_root.join("tracked.txt")).unwrap(),
        b"candidate change\n"
    );
}
