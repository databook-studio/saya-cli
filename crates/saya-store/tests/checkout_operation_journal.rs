use saya_store::{CodingEventAppend, NewCodingJob, SqliteStateStore};
use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1, GitObjectId,
    JobId, ManagedCheckoutId, ManagedCheckoutLifecycle, ManagedCheckoutPath,
    ManagedCheckoutRecordData, ManagedCheckoutRecordV1, RepositoryId, RepositoryMutationAction,
    Sha256Digest, SnapshotId,
};
use std::num::NonZeroU64;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "durable_state_contracts/checkout_operation_journal.rs"]
mod checkout_journal;

#[tokio::test]
async fn create_intent_is_written_as_one_durable_public_operation() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("saya-r14-create-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&database);
    let job_id = JobId::parse("job-r14-create").unwrap();
    let repository_id = RepositoryId::parse("repo-r14-create").unwrap();
    let snapshot_id = SnapshotId::parse("snapshot-r14-create").unwrap();
    store
        .create_coding_job(NewCodingJob {
            job_id: job_id.clone(),
            repository_id: repository_id.clone(),
            snapshot_id: snapshot_id.clone(),
        })
        .await
        .unwrap();
    let running = CodingEventV1::new(
        NonZeroU64::new(1).unwrap(),
        job_id.clone(),
        None,
        None,
        CodingEventKind::StateChanged {
            state: saya_types::JobState::Running,
        },
    )
    .unwrap();
    assert_eq!(
        store.append_coding_event(running).await.unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(1).unwrap())
    );

    let checkout_id = ManagedCheckoutId::parse("checkout-r14-create").unwrap();
    let checkout = ManagedCheckoutRecordV1::new(ManagedCheckoutRecordData {
        job_id: job_id.clone(),
        agent_id: None,
        managed_checkout_id: checkout_id.clone(),
        repository_id,
        common_dir_sha256: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        managed_path: ManagedCheckoutPath::new("/tmp/saya-r14-create").unwrap(),
        base_commit: GitObjectId::parse(&"b".repeat(40)).unwrap(),
        snapshot_id,
        snapshot_sha256: Sha256Digest::parse(&"c".repeat(64)).unwrap(),
        created_by_sequence: NonZeroU64::new(2).unwrap(),
        lifecycle: ManagedCheckoutLifecycle::Creating,
    })
    .unwrap();
    let operation = CheckoutOperationV1::new(
        job_id.clone(),
        NonZeroU64::new(2).unwrap(),
        checkout_id.clone(),
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Intent,
    )
    .unwrap();
    let event = CodingEventV1::new(
        NonZeroU64::new(2).unwrap(),
        job_id,
        None,
        None,
        CodingEventKind::CheckoutCreateIntent {
            checkout: Box::new(checkout),
            operation: Box::new(operation),
        },
    )
    .unwrap();
    let expected_ownership = match event.event() {
        CodingEventKind::CheckoutCreateIntent { checkout, .. } => checkout.as_ref().clone(),
        _ => unreachable!(),
    };

    assert_eq!(
        store
            .record_checkout_create_intent(event.clone())
            .await
            .unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(2).unwrap())
    );
    let terminal = CodingEventV1::new(
        NonZeroU64::new(3).unwrap(),
        JobId::parse("job-r14-create").unwrap(),
        None,
        None,
        CodingEventKind::StateChanged {
            state: saya_types::JobState::Completed,
        },
    )
    .unwrap();
    store.append_coding_event(terminal).await.unwrap();
    assert_eq!(
        store.record_checkout_create_intent(event).await.unwrap(),
        CodingEventAppend::AlreadyPresent(NonZeroU64::new(2).unwrap())
    );
    store.close().await;
    let reopened = SqliteStateStore::new(&database);
    assert_eq!(
        reopened
            .get_coding_job(&JobId::parse("job-r14-create").unwrap())
            .await
            .unwrap()
            .unwrap()
            .state,
        saya_types::JobState::Completed,
    );
    let persisted = reopened
        .get_managed_checkout(&JobId::parse("job-r14-create").unwrap(), &checkout_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.ownership, expected_ownership);
    assert_eq!(
        persisted.ownership.managed_path().as_str(),
        "/tmp/saya-r14-create"
    );
    assert_eq!(
        persisted.ownership.lifecycle(),
        ManagedCheckoutLifecycle::Creating
    );
    assert_eq!(
        persisted.current_lifecycle,
        ManagedCheckoutLifecycle::Creating
    );
    assert_eq!(
        persisted.latest_operation.state(),
        CheckoutOperationState::Intent
    );
    reopened.close().await;
    let _ = std::fs::remove_dir_all(root);
}
