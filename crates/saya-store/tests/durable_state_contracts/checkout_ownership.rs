use saya_store::{CodingEventAppend, NewCodingJob, SqliteStateStore, StoreError};
use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1, GitObjectId,
    JobId, ManagedCheckoutId, ManagedCheckoutLifecycle, ManagedCheckoutPath,
    ManagedCheckoutRecordData, ManagedCheckoutRecordV1, RepositoryId, RepositoryMutationAction,
    Sha256Digest, SnapshotId,
};
use std::{
    fs,
    num::NonZeroU64,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

struct Harness {
    root: PathBuf,
    store: SqliteStateStore,
}

impl Harness {
    async fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("saya-w05c3-{label}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        Self { root, store }
    }

    async fn seed_running_job(&self, tag: &str) -> (JobId, RepositoryId, SnapshotId) {
        let job = JobId::parse(&format!("job-w05c3-{tag}")).unwrap();
        let repository = RepositoryId::parse(&format!("repo-w05c3-{tag}")).unwrap();
        let snapshot = SnapshotId::parse(&format!("snapshot-w05c3-{tag}")).unwrap();
        self.store
            .create_coding_job(NewCodingJob {
                job_id: job.clone(),
                repository_id: repository.clone(),
                snapshot_id: snapshot.clone(),
            })
            .await
            .unwrap();
        let running = CodingEventV1::new(
            NonZeroU64::new(1).unwrap(),
            job.clone(),
            None,
            None,
            CodingEventKind::StateChanged {
                state: saya_types::JobState::Running,
            },
        )
        .unwrap();
        assert_eq!(
            self.store.append_coding_event(running).await.unwrap(),
            CodingEventAppend::Appended(NonZeroU64::new(1).unwrap())
        );
        (job, repository, snapshot)
    }

    async fn close(self) {
        self.store.close().await;
        let _ = fs::remove_dir_all(self.root);
    }
}

fn create_event(
    job: &JobId,
    sequence: u64,
    checkout_name: &str,
    repository: RepositoryId,
    snapshot: SnapshotId,
) -> CodingEventV1 {
    let id = ManagedCheckoutId::parse(checkout_name).unwrap();
    let sequence = NonZeroU64::new(sequence).unwrap();
    let checkout = ManagedCheckoutRecordV1::new(ManagedCheckoutRecordData {
        job_id: job.clone(),
        agent_id: None,
        managed_checkout_id: id.clone(),
        repository_id: repository,
        common_dir_sha256: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        managed_path: ManagedCheckoutPath::new(format!("/tmp/{checkout_name}")).unwrap(),
        base_commit: GitObjectId::parse(&"b".repeat(40)).unwrap(),
        snapshot_id: snapshot,
        snapshot_sha256: Sha256Digest::parse(&"c".repeat(64)).unwrap(),
        created_by_sequence: sequence,
        lifecycle: ManagedCheckoutLifecycle::Creating,
    })
    .unwrap();
    let operation = CheckoutOperationV1::new(
        job.clone(),
        sequence,
        id,
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Intent,
    )
    .unwrap();
    CodingEventV1::new(
        sequence,
        job.clone(),
        None,
        None,
        CodingEventKind::CheckoutCreateIntent {
            checkout: Box::new(checkout),
            operation: Box::new(operation),
        },
    )
    .unwrap()
}

#[tokio::test]
async fn cross_job_same_checkout_id_is_refused_and_writes_nothing() {
    let harness = Harness::new("cross-job").await;
    let (job_a, repository_a, snapshot_a) = harness.seed_running_job("a").await;
    let (job_b, repository_b, snapshot_b) = harness.seed_running_job("b").await;
    let shared = ManagedCheckoutId::parse("checkout-w05c3-shared").unwrap();
    let event_a = create_event(&job_a, 2, "checkout-w05c3-shared", repository_a, snapshot_a);
    assert_eq!(
        harness
            .store
            .record_checkout_create_intent(event_a)
            .await
            .unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(2).unwrap())
    );
    let event_b = create_event(&job_b, 2, "checkout-w05c3-shared", repository_b, snapshot_b);
    assert_eq!(
        harness
            .store
            .record_checkout_create_intent(event_b)
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    assert!(
        harness
            .store
            .list_managed_checkouts(&job_b, None, 10)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        harness.store.managed_checkout_owner(&shared).await.unwrap(),
        Some(job_a)
    );
    harness.close().await;
}

#[tokio::test]
async fn same_job_resend_stays_idempotent() {
    let harness = Harness::new("resend").await;
    let (job, repository, snapshot) = harness.seed_running_job("resend").await;
    let event = create_event(&job, 2, "checkout-w05c3-resend", repository, snapshot);
    assert_eq!(
        harness
            .store
            .record_checkout_create_intent(event.clone())
            .await
            .unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(2).unwrap())
    );
    assert_eq!(
        harness
            .store
            .record_checkout_create_intent(event)
            .await
            .unwrap(),
        CodingEventAppend::AlreadyPresent(NonZeroU64::new(2).unwrap())
    );
    harness.close().await;
}

#[tokio::test]
async fn managed_checkout_owner_reports_the_owning_job_or_none() {
    let harness = Harness::new("owner").await;
    let (job, repository, snapshot) = harness.seed_running_job("owner").await;
    let checkout = ManagedCheckoutId::parse("checkout-w05c3-owner").unwrap();
    let unknown = ManagedCheckoutId::parse("checkout-w05c3-unknown").unwrap();
    assert_eq!(
        harness
            .store
            .managed_checkout_owner(&checkout)
            .await
            .unwrap(),
        None
    );
    let event = create_event(&job, 2, "checkout-w05c3-owner", repository, snapshot);
    harness
        .store
        .record_checkout_create_intent(event)
        .await
        .unwrap();
    assert_eq!(
        harness
            .store
            .managed_checkout_owner(&checkout)
            .await
            .unwrap(),
        Some(job)
    );
    assert_eq!(
        harness
            .store
            .managed_checkout_owner(&unknown)
            .await
            .unwrap(),
        None
    );
    harness.close().await;
}

#[tokio::test]
async fn count_checkout_operations_counts_intents() {
    let harness = Harness::new("count").await;
    let (job, repository, snapshot) = harness.seed_running_job("count").await;
    assert_eq!(
        harness.store.count_checkout_operations(&job).await.unwrap(),
        0
    );
    for (sequence, name) in [
        (2u64, "checkout-w05c3-count-a"),
        (3u64, "checkout-w05c3-count-b"),
    ] {
        let event = create_event(&job, sequence, name, repository.clone(), snapshot.clone());
        harness
            .store
            .record_checkout_create_intent(event)
            .await
            .unwrap();
    }
    assert_eq!(
        harness.store.count_checkout_operations(&job).await.unwrap(),
        2
    );
    harness.close().await;
}
