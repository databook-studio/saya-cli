use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, NativeCodingJob,
    dispatch_coding,
};
use saya_store::ManagedCheckoutRecord;
use saya_types::{JobId, ManagedCheckoutId};
use std::sync::atomic::Ordering;

fn resolve_command(
    job: &str,
    candidate: &str,
    choice: &str,
    hash: &str,
    approve: bool,
) -> CodingCommand {
    CodingCommand::ResolveConflict {
        id: job.into(),
        candidate: candidate.into(),
        choice: choice.into(),
        record_sha256: hash.into(),
        approve,
    }
}

async fn checkout_count(fixture: &Fixture, job: &JobId) -> usize {
    fixture
        .store
        .list_managed_checkouts(job, None, 100)
        .await
        .unwrap()
        .items
        .len()
}

async fn resume(fixture: &Fixture, job: &JobId) -> std::sync::Arc<NativeCodingJob> {
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose_resume(job).await.unwrap())
            .unwrap();
    fixture
        .application
        .resume(job, &approval, CancellationToken::new())
        .await
        .unwrap()
}

async fn retain(job: &NativeCodingJob, candidate: &ManagedCheckoutId) {
    assert!(matches!(
        job.review(candidate).await,
        Err(CodingError::ConflictRetained { .. })
    ));
}

fn assert_candidate_matches(
    records: &[ManagedCheckoutRecord],
    source: &str,
    candidate: &ManagedCheckoutId,
) {
    let source = records
        .iter()
        .find(|record| record.ownership.managed_checkout_id().as_str() == source)
        .unwrap();
    let candidate = records
        .iter()
        .find(|record| record.ownership.managed_checkout_id() == candidate)
        .unwrap();
    assert_eq!(
        std::fs::read(
            std::path::Path::new(source.ownership.managed_path().as_str()).join("shared.txt"),
        )
        .unwrap(),
        std::fs::read(
            std::path::Path::new(candidate.ownership.managed_path().as_str()).join("shared.txt"),
        )
        .unwrap()
    );
}

#[tokio::test]
async fn explicit_resolution_revalidates_then_rebuilds_only_a_new_candidate() {
    let provider = Provider::pair_conflicts();
    let fixture =
        Fixture::with_endpoint("native-conflict-resolution", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("native-conflict-resolution").unwrap(),
        "Make dependent child changes".into(),
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
    assert!(!provider.overlap.load(Ordering::Acquire));

    let resumed = resume(&fixture, job.job_id()).await;
    let candidate = ManagedCheckoutId::parse("resolution-source").unwrap();
    retain(&resumed, &candidate).await;
    drop(resumed);
    let initial = fixture
        .application
        .conflicts(job.job_id(), &candidate)
        .await
        .unwrap();
    let initial_hash = initial["provenance"]["record_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    let stale_hash = format!(
        "{}{}",
        if initial_hash.starts_with('0') {
            "1"
        } else {
            "0"
        },
        &initial_hash[1..]
    );
    let count = checkout_count(&fixture, job.job_id()).await;

    assert!(matches!(
        dispatch_coding(
            &fixture.application,
            resolve_command(
                job.job_id().as_str(),
                candidate.as_str(),
                "keep-first",
                &initial_hash,
                false
            )
        )
        .await,
        Err(CodingError::ApprovalRequired)
    ));
    assert!(matches!(
        dispatch_coding(
            &fixture.application,
            resolve_command(
                job.job_id().as_str(),
                candidate.as_str(),
                "keep-first",
                &stale_hash,
                true
            )
        )
        .await,
        Err(CodingError::StaleReview)
    ));
    assert_eq!(checkout_count(&fixture, job.job_id()).await, count);

    let child_a = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 100)
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|record| {
            record
                .ownership
                .agent_id()
                .is_some_and(|agent| agent.as_str() == "child-a")
        })
        .unwrap();
    let child_a_root = std::path::Path::new(child_a.ownership.managed_path().as_str());
    std::fs::rename(
        child_a_root.join("rename-before.txt"),
        child_a_root.join("rename-after.txt"),
    )
    .unwrap();
    assert!(matches!(
        dispatch_coding(
            &fixture.application,
            resolve_command(
                job.job_id().as_str(),
                candidate.as_str(),
                "keep-first",
                &initial_hash,
                true
            )
        )
        .await,
        Err(CodingError::StaleReview)
    ));
    assert_eq!(checkout_count(&fixture, job.job_id()).await, count);

    let refreshed = resume(&fixture, job.job_id()).await;
    let fresh_candidate = ManagedCheckoutId::parse("resolution-fresh-source").unwrap();
    retain(&refreshed, &fresh_candidate).await;
    drop(refreshed);
    let retained = fixture
        .application
        .conflicts(job.job_id(), &fresh_candidate)
        .await
        .unwrap();
    let fresh_hash = retained["provenance"]["record_sha256"].as_str().unwrap();
    let winner = retained["conflict"]["inputs"]
        [retained["conflict"]["pairs"][0]["first"].as_u64().unwrap() as usize]["checkout_id"]
        .as_str()
        .unwrap();

    let result = dispatch_coding(
        &fixture.application,
        resolve_command(
            job.job_id().as_str(),
            fresh_candidate.as_str(),
            "keep-first",
            fresh_hash,
            true,
        ),
    )
    .await
    .unwrap();
    assert_eq!(result["review"]["apply_requires_fresh_approval"], true);
    let resolved = ManagedCheckoutId::parse(result["candidate"].as_str().unwrap()).unwrap();
    let records = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 100)
        .await
        .unwrap();
    assert_candidate_matches(&records.items, winner, &resolved);
    assert!(!fixture.repository.join("shared.txt").exists());
    assert_eq!(
        std::fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
    assert_eq!(
        std::fs::read(child_a_root.join("rename-after.txt")).unwrap(),
        b"stable rename bytes\n"
    );
    let second = dispatch_coding(
        &fixture.application,
        resolve_command(
            job.job_id().as_str(),
            fresh_candidate.as_str(),
            "keep-second",
            fresh_hash,
            true,
        ),
    )
    .await
    .unwrap();
    let second_candidate = ManagedCheckoutId::parse(second["candidate"].as_str().unwrap()).unwrap();
    let second_checkout = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 100)
        .await
        .unwrap();
    let second_winner = retained["conflict"]["inputs"]
        [retained["conflict"]["pairs"][0]["second"].as_u64().unwrap() as usize]["checkout_id"]
        .as_str()
        .unwrap();
    assert_candidate_matches(&second_checkout.items, second_winner, &second_candidate);
    assert!(!fixture.repository.join("shared.txt").exists());
}
