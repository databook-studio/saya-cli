//! Only the public checkout setup used by continuation owner controls.
use super::super::super::{
    event_projection_storage::event,
    job_metadata_storage::{create_job, temp_root},
};
use saya_store::SqliteStateStore;
use saya_types::{
    AgentId, CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1,
    GitObjectId, JobId, JobState, ManagedCheckoutId, ManagedCheckoutLifecycle, ManagedCheckoutPath,
    ManagedCheckoutRecordData, ManagedCheckoutRecordV1, RepositoryId, RepositoryMutationAction,
    Sha256Digest, SnapshotId,
};
use std::{fs, num::NonZeroU64, path::PathBuf};
pub struct Fixture {
    pub root: PathBuf,
    pub store: SqliteStateStore,
    pub job: JobId,
    pub repository: RepositoryId,
    pub snapshot: SnapshotId,
}
impl Fixture {
    pub async fn new(label: &str) -> Self {
        let root = temp_root(label);
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let job = create_job(&store, label).await;
        store
            .append_coding_event(event(
                1,
                &job.job_id,
                None,
                CodingEventKind::StateChanged {
                    state: JobState::Running,
                },
            ))
            .await
            .unwrap();
        Self {
            root,
            store,
            job: job.job_id,
            repository: job.repository_id,
            snapshot: job.snapshot_id,
        }
    }
    pub fn create_event(
        &self,
        sequence: u64,
        name: &str,
        repository: RepositoryId,
        snapshot: SnapshotId,
        owner: Option<AgentId>,
    ) -> CodingEventV1 {
        let sequence = NonZeroU64::new(sequence).unwrap();
        let id = ManagedCheckoutId::parse(name).unwrap();
        let checkout = ManagedCheckoutRecordV1::new(ManagedCheckoutRecordData {
            job_id: self.job.clone(),
            agent_id: owner.clone(),
            managed_checkout_id: id.clone(),
            repository_id: repository,
            common_dir_sha256: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
            managed_path: ManagedCheckoutPath::new(format!("/tmp/{name}")).unwrap(),
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
        name: &str,
        action: RepositoryMutationAction,
        state: CheckoutOperationState,
        owner: Option<AgentId>,
    ) -> CodingEventV1 {
        let operation = CheckoutOperationV1::new(
            self.job.clone(),
            NonZeroU64::new(operation_sequence).unwrap(),
            ManagedCheckoutId::parse(name).unwrap(),
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
    pub async fn close(self) {
        self.store.close().await;
        fs::remove_dir_all(self.root).unwrap();
    }
}
