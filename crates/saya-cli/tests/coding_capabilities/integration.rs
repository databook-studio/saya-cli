use super::{Fixture, Provider, prepared_candidate, results};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;

#[tokio::test]
async fn coordinator_assembles_completed_child_changes_and_reviews_without_apply() {
    let provider = Provider::integrated_review();
    let fixture = Fixture::with_endpoint("assembled-review", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("assembled-review").unwrap(),
        "Implement and review the child change".into(),
        vec!["tracked.txt".into()],
    )
    .unwrap()
    .with_tools_allow(Some(vec![
        "helpers".to_owned(),
        "read".to_owned(),
        "write".to_owned(),
    ]))
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), job.run(&NoopEventSink))
        .await
        .unwrap();
    let children = fixture
        .store
        .list_coding_children(job.job_id(), None, 100)
        .await
        .unwrap();
    assert_eq!(children.items.len(), 1);
    assert_eq!(
        children.items[0].state,
        saya_types::JobState::Completed,
        "child transport must complete before assembly is evaluated"
    );
    let writes = results(&provider, "write");
    assert!(
        writes
            .iter()
            .any(|result| result.contains("digest") && !result.contains("error")),
        "actual child write must succeed: {writes:?}"
    );
    // Helpers write the root's own working checkout, so the child's change is
    // already in W when the owner prepares the candidate from it: there is no
    // per-child checkout and no merge step to observe.
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 8)
        .await
        .unwrap();
    let working = checkouts
        .items
        .iter()
        .find(|record| {
            record.ownership.agent_id().is_none()
                && record
                    .ownership
                    .managed_checkout_id()
                    .as_str()
                    .starts_with("work-")
        })
        .expect("the root working checkout W exists");
    assert_eq!(
        std::fs::read_to_string(
            std::path::Path::new(working.ownership.managed_path().as_str()).join("tracked.txt")
        )
        .unwrap(),
        "child-assembled-content\n",
        "the helper's change landed in the shared working checkout"
    );
    assert!(outcome.is_ok(), "run failed: {outcome:?}");
    let candidate = prepared_candidate(&fixture, job.job_id()).await;
    // The operator reviews through the same path `coding review` uses: resume
    // the job (a fresh owner), then review the prepared candidate.
    let proposal = fixture
        .application
        .propose_resume(job.job_id())
        .await
        .unwrap();
    let approval = CodingAuthorization::scoped(&proposal).unwrap();
    let resumed = fixture
        .application
        .resume(job.job_id(), &approval, CancellationToken::new())
        .await
        .unwrap();
    let review: serde_json::Value = resumed
        .review_existing(&candidate)
        .await
        .unwrap()
        .preview()
        .clone();
    assert_eq!(
        review["changes"][0]["path"], "tracked.txt",
        "actual review: {review}"
    );
    assert!(
        review["diff"]["files"][0]["after"]
            .as_str()
            .unwrap()
            .contains("child-assembled-content")
    );
    assert_eq!(
        std::fs::read_to_string(
            std::path::Path::new(
                fixture
                    .store
                    .get_managed_checkout(job.job_id(), &candidate)
                    .await
                    .unwrap()
                    .unwrap()
                    .ownership
                    .managed_path()
                    .as_str()
            )
            .join("tracked.txt")
        )
        .unwrap(),
        "child-assembled-content\n",
        "the prepared candidate carries the child's change"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.repository.join("tracked.txt")).unwrap(),
        "original\n"
    );
    fixture
        .application
        .propose_resume(job.job_id())
        .await
        .unwrap();
}
