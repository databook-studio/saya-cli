use saya_store::{CodingEventAppend, NewCodingJob, SqliteStateStore};
use saya_types::{
    AgentId, CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1,
    GitObjectId, JobId, ManagedCheckoutId, ManagedCheckoutLifecycle, ManagedCheckoutPath,
    ManagedCheckoutRecordData, ManagedCheckoutRecordV1, RepositoryId, RepositoryMutationAction,
    Sha256Digest, SnapshotId,
};
use std::{
    fs,
    num::NonZeroU64,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Fixture {
    pub root: PathBuf,
    pub database: PathBuf,
    pub store: SqliteStateStore,
    pub job: JobId,
    pub repository: RepositoryId,
    pub snapshot: SnapshotId,
}

pub async fn assert_lifecycle(fixture: &Fixture, name: &str, expected: ManagedCheckoutLifecycle) {
    let checkout = fixture
        .store
        .get_managed_checkout(&fixture.job, &ManagedCheckoutId::parse(name).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checkout.current_lifecycle, expected);
}

impl Fixture {
    pub async fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("saya-r14-{label}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let database = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&database);
        let job = JobId::parse(&format!("job-r14-{label}")).unwrap();
        let repository = RepositoryId::parse(&format!("repo-r14-{label}")).unwrap();
        let snapshot = SnapshotId::parse(&format!("snapshot-r14-{label}")).unwrap();
        store
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
            store.append_coding_event(running).await.unwrap(),
            CodingEventAppend::Appended(NonZeroU64::new(1).unwrap())
        );
        Self {
            root,
            database,
            store,
            job,
            repository,
            snapshot,
        }
    }

    pub fn create_event(
        &self,
        sequence: u64,
        checkout_name: &str,
        repository: RepositoryId,
        snapshot: SnapshotId,
        owner: Option<AgentId>,
    ) -> CodingEventV1 {
        let id = ManagedCheckoutId::parse(checkout_name).unwrap();
        let sequence = NonZeroU64::new(sequence).unwrap();
        let checkout = ManagedCheckoutRecordV1::new(ManagedCheckoutRecordData {
            job_id: self.job.clone(),
            agent_id: owner.clone(),
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
            self.job.clone(),
            sequence,
            id,
            RepositoryMutationAction::CreateCheckout,
            CheckoutOperationState::Intent,
        )
        .unwrap();
        CodingEventV1::new(
            sequence,
            self.job.clone(),
            owner,
            None,
            CodingEventKind::CheckoutCreateIntent {
                checkout: Box::new(checkout),
                operation: Box::new(operation),
            },
        )
        .unwrap()
    }

    pub fn operation_event(
        &self,
        event_sequence: u64,
        operation_sequence: u64,
        checkout_name: &str,
        action: RepositoryMutationAction,
        state: CheckoutOperationState,
        owner: Option<AgentId>,
    ) -> CodingEventV1 {
        let operation = CheckoutOperationV1::new(
            self.job.clone(),
            NonZeroU64::new(operation_sequence).unwrap(),
            ManagedCheckoutId::parse(checkout_name).unwrap(),
            action,
            state,
        )
        .unwrap();
        let event = if state == CheckoutOperationState::Intent {
            CodingEventKind::CheckoutOperationIntent {
                operation: Box::new(operation),
            }
        } else {
            CodingEventKind::CheckoutOperationOutcome {
                operation: Box::new(operation),
            }
        };
        CodingEventV1::new(
            NonZeroU64::new(event_sequence).unwrap(),
            self.job.clone(),
            owner,
            None,
            event,
        )
        .unwrap()
    }

    pub async fn create(&self, sequence: u64, name: &str) -> CodingEventV1 {
        let event = self.create_event(
            sequence,
            name,
            self.repository.clone(),
            self.snapshot.clone(),
            None,
        );
        self.store
            .record_checkout_create_intent(event.clone())
            .await
            .unwrap();
        event
    }

    pub async fn complete_create(&self, event_sequence: u64, operation_sequence: u64, name: &str) {
        let event = self.operation_event(
            event_sequence,
            operation_sequence,
            name,
            RepositoryMutationAction::CreateCheckout,
            CheckoutOperationState::Completed,
            None,
        );
        self.store
            .record_checkout_operation_outcome(event)
            .await
            .unwrap();
    }

    pub async fn close(self) {
        self.store.close().await;
        let _ = fs::remove_dir_all(self.root);
    }
}
