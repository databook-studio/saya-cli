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
async fn native_review_shows_complete_current_diagnostic_evidence() {
    let output = r#"{"reason":"compiler-message","message":{"message":"unused variable x","code":{"code":"unused_variables"},"level":"warning","spans":[{"file_name":"tracked.txt","line_start":1,"line_end":1,"column_start":1,"column_end":2,"is_primary":true}]}}"#;
    let fixture = Fixture::with_diagnostic_output("review-diagnostics", output).await;
    let request = CodingJobRequest::new(
        JobId::parse("job-review-diagnostics").unwrap(),
        "review diagnostics".into(),
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
    let candidate = ManagedCheckoutId::parse("review-diagnostic-candidate").unwrap();
    job.create_checkout(&candidate).await.unwrap();
    let review = job.review_existing(&candidate).await.unwrap();
    let preview = review.preview();
    let check = &preview["checks"][0];

    assert_eq!(check["outcome"]["status"], "known_exit");
    assert_eq!(check["process"]["exit_code"], 0);
    assert_eq!(check["successful"], true);
    assert_eq!(check["diagnostic_format"], "cargo_json");
    assert_eq!(check["diagnostics_state"], "complete");
    assert_eq!(check["diagnostics_complete"], true);
    assert_eq!(check["diagnostic_count"], 1);
    assert_eq!(check["diagnostics"][0]["severity"], "Warning");
    assert_eq!(check["diagnostics"][0]["message"], "unused variable x");
    assert_eq!(
        check["diagnostics"][0]["locations"][0]["path"],
        "tracked.txt"
    );
    assert_eq!(check["diagnostic_delta"]["available"], false);
    assert_eq!(
        review.preview()["diagnostic_delta"]["reason"],
        "comparable_baseline_not_minted"
    );
}

#[tokio::test]
async fn native_review_keeps_process_evidence_when_diagnostics_cannot_parse() {
    let fixture =
        Fixture::with_diagnostic_output("review-malformed-diagnostics", "not-json\n").await;
    let request = CodingJobRequest::new(
        JobId::parse("job-review-malformed").unwrap(),
        "review malformed diagnostics".into(),
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
    let candidate = ManagedCheckoutId::parse("review-malformed-candidate").unwrap();
    job.create_checkout(&candidate).await.unwrap();
    let review = job.review_existing(&candidate).await.unwrap();
    let preview = review.preview();
    let check = &preview["checks"][0];

    assert_eq!(check["process"]["exit_code"], 0);
    assert_eq!(check["successful"], true);
    assert_eq!(check["diagnostic_format"], "cargo_json");
    assert_eq!(check["diagnostics_state"], "unavailable");
    assert_eq!(check["diagnostic_unavailable_reason"], "parse_failed");
    assert_eq!(check["diagnostics_complete"], false);
    assert_eq!(check["diagnostic_count"], serde_json::Value::Null);
    assert_eq!(check["diagnostics"], serde_json::Value::Null);
    assert_eq!(review.preview()["diagnostic_delta"]["available"], false);
}
