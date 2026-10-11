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
async fn native_review_mints_then_reuses_a_diagnostic_baseline() {
    let output = r#"{"reason":"compiler-message","message":{"message":"password=z tail","code":{"code":"secret-shaped"},"level":"warning","spans":[{"file_name":"tracked.txt","line_start":1,"line_end":1,"column_start":1,"column_end":2,"is_primary":true}]}}
{"reason":"compiler-message","message":{"message":"password=zz","code":{"code":"secret-shaped"},"level":"warning","spans":[{"file_name":"tracked.txt","line_start":1,"line_end":1,"column_start":1,"column_end":2,"is_primary":true}]}}"#;
    let fixture = Fixture::with_diagnostic_output("review-baseline", output).await;
    let request = CodingJobRequest::new(
        JobId::parse("job-review-baseline").unwrap(),
        "review diagnostics baseline".into(),
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
    let candidate = ManagedCheckoutId::parse("review-baseline-candidate").unwrap();
    job.create_checkout(&candidate).await.unwrap();

    let first = job.review_existing(&candidate).await.unwrap();
    assert_eq!(
        first.preview()["diagnostic_delta"],
        serde_json::json!({
            "available": false,
            "reason": "baseline_minted_for_future_review"
        })
    );
    let review_records = fixture
        .root
        .join("coding")
        .join(job.job_id().as_str())
        .join(format!("review-{}.jsonl", candidate.as_str()));
    let records = std::fs::read_to_string(review_records).unwrap();
    assert!(records.contains("baseline_pending_publication"));
    assert!(records.contains("baseline_minted_for_future_review"));

    let second = job.review_existing(&candidate).await.unwrap();
    assert_eq!(second.preview()["diagnostic_delta"]["available"], true);
    assert_eq!(second.preview()["diagnostic_delta"]["introduced_count"], 0);
    assert_eq!(second.preview()["diagnostic_delta"]["unchanged_count"], 2);
    assert_eq!(second.preview()["diagnostic_delta"]["resolved_count"], 0);
    assert_eq!(
        second.preview()["checks"][0]["diagnostic_delta"]["unchanged"][0]["message"],
        "password=[redacted]"
    );
    assert_eq!(
        second.preview()["checks"][0]["diagnostic_delta"]["unchanged"][1]["message"],
        "password=[redacted] tail"
    );
}

#[tokio::test]
async fn corrupt_diagnostic_baseline_stays_unavailable_and_is_not_overwritten() {
    let output = r#"{"reason":"compiler-message","message":{"message":"password=hunter2","code":{"code":"unused_variables"},"level":"warning","spans":[{"file_name":"tracked.txt","line_start":1,"line_end":1,"column_start":1,"column_end":2,"is_primary":true}]}}"#;
    let fixture = Fixture::with_diagnostic_output("review-baseline-corrupt", output).await;
    let request = CodingJobRequest::new(
        JobId::parse("job-review-baseline-corrupt").unwrap(),
        "review corrupt diagnostic baseline".into(),
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
    let candidate = ManagedCheckoutId::parse("review-corrupt-candidate").unwrap();
    job.create_checkout(&candidate).await.unwrap();
    job.review_existing(&candidate).await.unwrap();

    let records = fixture.root.join("coding").join(job.job_id().as_str());
    let baseline = std::fs::read_dir(&records)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("checks-diagnostic-baseline-")
                        && !name.starts_with("checks-diagnostic-baseline-commit-")
                })
        })
        .expect("review should persist a diagnostic baseline");
    let publication = std::fs::read_dir(&records)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("checks-diagnostic-baseline-commit-"))
        })
        .expect("review should persist a diagnostic baseline publication marker");
    let original = std::fs::read(&baseline).unwrap();
    assert!(!String::from_utf8_lossy(&original).contains("hunter2"));
    std::fs::write(&baseline, b"tampered").unwrap();

    let review = job.review_existing(&candidate).await.unwrap();
    assert_eq!(review.preview()["diagnostic_delta"]["available"], false);
    assert_eq!(
        review.preview()["diagnostic_delta"]["reason"],
        "baseline_unavailable"
    );
    assert_eq!(std::fs::read(&baseline).unwrap(), b"tampered");
    assert!(publication.exists());
    std::fs::write(&baseline, &original).unwrap();
    std::fs::remove_file(&publication).unwrap();
    let review = job.review_existing(&candidate).await.unwrap();
    assert_eq!(
        review.preview()["diagnostic_delta"]["reason"],
        "baseline_unavailable"
    );
    assert_eq!(std::fs::read(&baseline).unwrap(), original);
}

#[tokio::test]
async fn native_review_keeps_process_evidence_when_diagnostics_cannot_parse() {
    let fixture =
        Fixture::with_diagnostic_output("review-malformed-diagnostics", "not-json\n").await;
    let request = CodingJobRequest::new(
        JobId::parse("job-review-malformed").unwrap(),
        "review malformed diagnostics".into(),
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
