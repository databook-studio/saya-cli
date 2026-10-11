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
        vec![
            "shared.txt".into(),
            "rename-before.txt".into(),
            "rename-after.txt".into(),
        ],
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
            .map(|content| content.trim().to_owned())
            .collect::<std::collections::HashSet<_>>(),
        (0..24)
            .map(|index| format!("child-{index:02}"))
            .collect::<std::collections::HashSet<_>>()
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
    assert_eq!(inspected["status"], "retained");
    assert_eq!(
        inspected["current_state"],
        "unknown_requires_fresh_revalidation"
    );
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
    assert_eq!(inspected["conflict"]["version"], 2);
    assert_eq!(inspected["provenance"]["complete"], true);
    assert_eq!(inspected["provenance"]["source_sha256s_complete"], true);
    assert_eq!(
        inspected["provenance"]["record_sha256_scope"],
        "canonical_record_without_record_sha256"
    );
    assert_eq!(
        inspected["provenance"]["record_sha256"],
        inspected["conflict"]["record_sha256"]
    );
    let first_conflict = inspected["conflict"].clone();
    let child_zero = records
        .items
        .iter()
        .find(|record| {
            record
                .ownership
                .agent_id()
                .is_some_and(|agent| agent.as_str() == "child-00")
        })
        .unwrap();
    let child_zero_root = std::path::Path::new(child_zero.ownership.managed_path().as_str());
    std::fs::rename(
        child_zero_root.join("rename-before.txt"),
        child_zero_root.join("rename-after.txt"),
    )
    .unwrap();
    let next_candidate = ManagedCheckoutId::parse("conflict-candidate-next").unwrap();
    assert!(matches!(
        resumed.review(&next_candidate).await,
        Err(CodingError::ConflictRetained { .. })
    ));
    let next_inspected = fixture
        .application
        .conflicts(job.job_id(), &next_candidate)
        .await
        .unwrap();
    let next_conflict = &next_inspected["conflict"];
    let child_zero_id = child_zero.ownership.managed_checkout_id().as_str();
    let first_child_zero = first_conflict["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["checkout_id"] == child_zero_id)
        .unwrap();
    let next_child_zero = next_conflict["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["checkout_id"] == child_zero_id)
        .unwrap();
    assert_eq!(first_conflict["pairs"], next_conflict["pairs"]);
    assert_ne!(
        first_conflict["candidate_checkout_id"],
        next_conflict["candidate_checkout_id"]
    );
    assert_eq!(
        first_conflict["candidate_source_sha256"],
        next_conflict["candidate_source_sha256"]
    );
    assert_ne!(
        first_child_zero["source_sha256"],
        next_child_zero["source_sha256"]
    );
    assert_ne!(
        first_conflict["record_sha256"],
        next_conflict["record_sha256"]
    );
    assert_eq!(
        std::fs::read(child_zero_root.join("rename-after.txt")).unwrap(),
        b"stable rename bytes\n"
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
