use saya_agent::{AgentLimits, AgentRequest, AnsweringLimits};
use saya_harness::coding::{
    budget::{CountLimits, JobBudgetLedger},
    child::ChildExecutionSpec,
    policy::{
        ChildAuthorityRequest, ChildRole, PlatformSupport, RootDelegationInput, RootDelegationScope,
    },
};
use saya_store::{NewCodingChild, NewCodingJob, SqliteStateStore};
use saya_types::{
    AgentId, Capabilities, CodingEventKind, CodingEventV1, InvocationId, JobId, JobState,
    RepositoryId, Sha256Digest, SnapshotId,
};
use std::{
    fs,
    num::NonZeroU64,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Fixture {
    pub root: PathBuf,
    pub database: PathBuf,
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
            "saya-r03a-{label}-{}-{stamp}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root).unwrap();
        let database = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&database);
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
            database,
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
        self.authority(agent_id, invocation_id).await
    }

    pub async fn authority(
        &self,
        agent_id: AgentId,
        invocation_id: InvocationId,
    ) -> ChildAuthorityView {
        RootDelegationScope::approve(
            RootDelegationInput {
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
            },
            &FixtureApproval,
        )
        .await
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

    pub fn spec(
        &self,
        authority: ChildAuthorityView,
        prompt: &str,
        max_requests: u64,
        max_tool_calls: u64,
    ) -> ChildExecutionSpec {
        let agent_tool_calls =
            usize::try_from(max_tool_calls).expect("tool-call ceiling exceeds usize");
        ChildExecutionSpec::new(
            authority,
            AgentRequest {
                prompt: prompt.to_owned(),
                profile_names: Vec::new(),
                model: "r03a-test-model".to_owned(),
                system_prompt: None,
                history: Vec::new(),
                context_blocks: Vec::new(),
            },
            AgentLimits {
                max_turns: Some(4),
                max_tool_calls: Some(agent_tool_calls),
                max_continuations: Some(1),
                permit_candidate_writes: false,
                permit_workspace_writes: false,
                permit_external_effects: false,
                context_byte_budget: 64 * 1024,
            },
            AnsweringLimits {
                max_requests: Some(max_requests),
                max_tool_calls: Some(max_tool_calls),
                max_reported_tokens: Some(10_000),
            },
            None,
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
}

pub(super) use saya_harness::coding::policy::ChildAuthorityView;

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

struct FixtureApproval;
#[async_trait::async_trait]
impl saya_agent::ApprovalDecider for FixtureApproval {
    async fn approve(&self, _: &saya_agent::ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}
