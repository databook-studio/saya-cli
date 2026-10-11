use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingError, CodingJobRequest};
use saya_types::{JobId, ManagedCheckoutId};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn sequential_same_path_overlap_is_retained_and_inspectable_without_mutation() {
    let provider = Provider::conflicts();
    let fixture = Fixture::with_endpoint("native-conflicts", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("native-conflicts").unwrap(),
        "Make dependent native child changes".into(),
        vec!["shared.txt".into()],
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
    let output = job.run(&NoopEventSink).await.unwrap();
    assert!(!output.truncated);
    assert!(
        !provider.overlap.load(Ordering::Acquire),
        "same-path children ran sequentially"
    );
    let approval = CodingAuthorization::scoped(
        &fixture
            .application
            .propose_resume(job.job_id())
            .await
            .unwrap(),
    )
    .unwrap();
    let resumed = fixture
        .application
        .resume(job.job_id(), &approval, CancellationToken::new())
        .await
        .unwrap();
    let candidate = ManagedCheckoutId::parse("conflict-candidate").unwrap();
    assert!(matches!(
        resumed.review(&candidate).await,
        Err(CodingError::ConflictRetained { .. })
    ));

    let records = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 100)
        .await
        .unwrap();
    let child_paths: Vec<_> = records
        .items
        .iter()
        .filter(|record| record.ownership.agent_id().is_some())
        .map(|record| {
            std::path::Path::new(record.ownership.managed_path().as_str()).join("shared.txt")
        })
        .collect();
    assert_eq!(child_paths.len(), 24);
    let contents: Vec<_> = child_paths
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect();
    assert_eq!(
        contents
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        24
    );
    assert!(!fixture.repository.join("shared.txt").exists());
    assert_eq!(
        std::fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
    let candidate_record = records
        .items
        .iter()
        .find(|record| record.ownership.managed_checkout_id() == &candidate)
        .unwrap();
    assert!(
        !std::path::Path::new(candidate_record.ownership.managed_path().as_str())
            .join("shared.txt")
            .exists()
    );
    assert_eq!(
        std::fs::read(
            std::path::Path::new(candidate_record.ownership.managed_path().as_str())
                .join("tracked.txt")
        )
        .unwrap(),
        b"original\n"
    );

    let inspected = fixture
        .application
        .conflicts(job.job_id(), &candidate)
        .await
        .unwrap();
    assert_eq!(inspected["status"], "open");
    assert_eq!(inspected["authority"], "none");
    assert_eq!(inspected["resolution"], "not_implemented");
    assert_eq!(inspected["fresh_revalidation_required"], true);
    assert_eq!(
        inspected["conflict"]["pairs"].as_array().unwrap().len(),
        276
    );
    assert_eq!(inspected["conflict"]["pairs"][0]["path"], "shared.txt");
    assert_eq!(
        inspected["conflict"]["inputs"].as_array().unwrap().len(),
        24
    );
    let metadata = fixture
        .root
        .join("coding")
        .join(job.job_id().as_str())
        .join("conflicts.jsonl");
    let tampered = std::fs::read_to_string(&metadata).unwrap().replace(
        "\"job_id\":\"native-conflicts\"",
        "\"job_id\":\"foreign-job\"",
    );
    std::fs::write(metadata, tampered).unwrap();
    assert!(
        fixture
            .application
            .conflicts(job.job_id(), &candidate)
            .await
            .is_err()
    );
}
