use std::{
    fs,
    num::NonZeroU64,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use saya_harness::coding::budget::{
    AppendedEvent, ChildCountLimits, CountLimits, JobBudgetLedger, JobEventSequencer,
    SequencerError,
};
use saya_harness::coding::policy::{
    ChildAuthorityRequest, ChildAuthorityView, ChildRole, PlatformSupport, RootDelegationInput,
    RootDelegationScope,
};
use saya_store::{NewCodingChild, NewCodingJob, SqliteStateStore};
use saya_types::{
    AgentId, Capabilities, CodingEventKind, CodingEventV1, CodingResultOutcome, InvocationId,
    JobId, JobState, RepositoryId, Sha256Digest, SnapshotId,
};

pub(super) const CHILD_LIMITS: ChildCountLimits = ChildCountLimits {
    logical_requests: 2,
    tool_calls: 1,
};

pub(super) fn job_event(job: &JobId, sequence: u64, kind: CodingEventKind) -> CodingEventV1 {
    CodingEventV1::new(
        NonZeroU64::new(sequence).unwrap(),
        job.clone(),
        None,
        None,
        kind,
    )
    .unwrap()
}

pub(super) fn child_event(
    job: &JobId,
    actor: (&AgentId, &InvocationId),
    sequence: u64,
    kind: CodingEventKind,
) -> CodingEventV1 {
    CodingEventV1::new(
        NonZeroU64::new(sequence).unwrap(),
        job.clone(),
        Some(actor.0.clone()),
        Some(actor.1.clone()),
        kind,
    )
    .unwrap()
}

/// Local append helpers over one sequencer view: each closure fills the
/// allocated sequence into its event before any store contact.
pub(super) struct Appends<'a> {
    pub(super) sequencer: &'a JobEventSequencer,
    pub(super) job: &'a JobId,
}

impl Appends<'_> {
    pub(super) async fn child_state(
        &self,
        actor: (&AgentId, &InvocationId),
        state: JobState,
    ) -> AppendedEvent {
        self.sequencer
            .append_event(|sequence| {
                child_event(
                    self.job,
                    actor,
                    sequence,
                    CodingEventKind::StateChanged { state },
                )
            })
            .await
            .unwrap()
    }

    pub(super) async fn child_result(&self, actor: (&AgentId, &InvocationId)) -> AppendedEvent {
        self.sequencer
            .append_event(|sequence| {
                child_event(
                    self.job,
                    actor,
                    sequence,
                    CodingEventKind::ResultReported {
                        source_sha256: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
                        outcome: CodingResultOutcome::Completed,
                    },
                )
            })
            .await
            .unwrap()
    }

    pub(super) async fn job_state(&self, state: JobState) -> Result<AppendedEvent, SequencerError> {
        self.sequencer
            .append_event(|sequence| {
                job_event(self.job, sequence, CodingEventKind::StateChanged { state })
            })
            .await
    }
}

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Fixture {
    pub root: PathBuf,
    pub store: SqliteStateStore,
    pub job: JobId,
    pub repository: RepositoryId,
    pub snapshot: SnapshotId,
}

impl Fixture {
    pub async fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "saya-r03b-{label}-{}-{stamp}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root).unwrap();
        let store = SqliteStateStore::new(&root.join("state.sqlite3"));
        let job = JobId::parse("job-a").unwrap();
        let repository = RepositoryId::parse("repository-a").unwrap();
        let snapshot = SnapshotId::parse("snapshot-a").unwrap();
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
                state: JobState::Running,
            },
        )
        .unwrap();
        store.append_coding_event(running).await.unwrap();
        Self {
            root,
            store,
            job,
            repository,
            snapshot,
        }
    }

    pub async fn add_child(&self, agent: &str, invocation: &str) -> ChildAuthorityView {
        let agent_id = AgentId::parse(agent).unwrap();
        let invocation_id = InvocationId::parse(invocation).unwrap();
        self.store
            .create_coding_child(NewCodingChild {
                job_id: self.job.clone(),
                agent_id: agent_id.clone(),
                current_invocation_id: invocation_id.clone(),
            })
            .await
            .unwrap();
        self.authority(agent_id, invocation_id)
    }

    fn authority(&self, agent_id: AgentId, invocation_id: InvocationId) -> ChildAuthorityView {
        RootDelegationScope::new(RootDelegationInput {
            job_id: self.job.clone(),
            agent_id,
            invocation_id,
            repository_id: self.repository.clone(),
            snapshot_id: self.snapshot.clone(),
            managed_checkout_id: None,
            policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
            revocation_epoch: 1,
            write_paths: Vec::new(),
            capabilities: Capabilities::default(),
            database_profiles: Vec::new(),
        })
        .unwrap()
        .narrow(
            ChildAuthorityRequest::new(
                ChildRole::Analyst,
                Vec::new(),
                Capabilities::default(),
                Vec::new(),
            )
            .unwrap(),
            PlatformSupport::default(),
        )
        .unwrap()
    }

    pub fn ledger(
        &self,
        limits: CountLimits,
    ) -> impl std::future::Future<
        Output = Result<JobBudgetLedger, saya_harness::coding::budget::BudgetLedgerError>,
    > + '_ {
        JobBudgetLedger::new(self.store.clone(), self.job.clone(), limits)
    }

    pub async fn close(self) {
        self.store.close().await;
        let _ = fs::remove_dir_all(self.root);
    }

    /// Closes the fixture's store handle but keeps the directory, so a
    /// test can reopen the same database as a restarted process.
    pub async fn close_store(&self) {
        self.store.close().await;
    }

    pub fn reopen(&self) -> SqliteStateStore {
        SqliteStateStore::new(&self.root.join("state.sqlite3"))
    }
}

pub(super) fn limits(
    logical_requests: u64,
    transport_attempts: u64,
    tool_calls: u64,
    model_slots: u64,
    child_slots: u64,
    deadline: Duration,
) -> CountLimits {
    CountLimits {
        logical_requests,
        transport_attempts,
        tool_calls,
        model_slots,
        child_slots,
        deadline,
    }
}
