use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use saya_agent::{AgentLimits, AgentRequest, AllowReadOnlyApproval, AnsweringLimits};
use saya_harness::coding::{
    child::{ChildComponents, ChildExecutionSpec, ChildFactory, ChildFactoryError},
    policy::{
        ChildAuthorityRequest, ChildRole, CommandCheckoutProfile, PlatformSupport,
        RootDelegationInput,
    },
};
use saya_types::{
    AgentId, Capabilities, InvocationId, JobId, ManagedCheckoutId, RepositoryId, Sha256Digest,
    SnapshotId, WorkspacePath,
};

#[path = "a04_support/loop_parts.rs"]
mod loop_parts;
#[path = "a04_support/request_bounds.rs"]
mod request_bounds;
use loop_parts::{CountingExecutor, ScriptedProvider, inspection_tool};

pub const MODEL: &str = "test-model";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedAuthority {
    pub job_id: String,
    pub agent_id: String,
    pub invocation_id: String,
    pub repository_id: String,
    pub snapshot_id: String,
    pub checkout_id: Option<String>,
    pub write_paths: Vec<String>,
    pub workspace_write: bool,
}

#[derive(Default)]
pub struct Stats {
    pub factory_calls: AtomicUsize,
    pub provider_calls: AtomicUsize,
    pub executor_calls: AtomicUsize,
    pub captured: Mutex<Vec<CapturedAuthority>>,
}

pub struct ScenarioFactory {
    pub stats: Arc<Stats>,
    pub first_turn_tool_calls: usize,
    pub text_deltas: usize,
}

impl ScenarioFactory {
    pub fn new(first_turn_tool_calls: usize) -> Self {
        Self {
            stats: Arc::new(Stats::default()),
            first_turn_tool_calls,
            text_deltas: 0,
        }
    }

    pub fn with_text_deltas(text_deltas: usize) -> Self {
        Self {
            stats: Arc::new(Stats::default()),
            first_turn_tool_calls: 0,
            text_deltas,
        }
    }
}

impl ChildFactory for ScenarioFactory {
    // This scripted factory creates no process runner; the coordinator still
    // receives and uses the invocation token for its agent loop.
    fn build(
        &self,
        spec: &ChildExecutionSpec,
        _cancellation: &saya_agent::CancellationToken,
    ) -> Result<ChildComponents, ChildFactoryError> {
        self.stats.factory_calls.fetch_add(1, Ordering::SeqCst);
        let authority = spec.authority();
        self.stats.captured.lock().unwrap().push(CapturedAuthority {
            job_id: authority.job_id().as_str().to_owned(),
            agent_id: authority.agent_id().as_str().to_owned(),
            invocation_id: authority.invocation_id().as_str().to_owned(),
            repository_id: authority.repository_id().as_str().to_owned(),
            snapshot_id: authority.snapshot_id().as_str().to_owned(),
            checkout_id: authority
                .managed_checkout_id()
                .map(|checkout| checkout.as_str().to_owned()),
            write_paths: authority
                .write_paths()
                .iter()
                .map(|path| path.as_str().to_owned())
                .collect(),
            workspace_write: authority.capabilities().workspace_write,
        });
        Ok(ChildComponents::new(
            Box::new(ScriptedProvider {
                stats: Arc::clone(&self.stats),
                first_turn_tool_calls: self.first_turn_tool_calls,
                text_deltas: self.text_deltas,
                turn_calls: AtomicUsize::new(0),
            }),
            Box::new(CountingExecutor(Arc::clone(&self.stats))),
            vec![inspection_tool()],
            Box::new(AllowReadOnlyApproval),
        ))
    }
}

pub fn authority_view(
    agent: &str,
    invocation: &str,
) -> saya_harness::coding::policy::ChildAuthorityView {
    let checkout = ManagedCheckoutId::parse("checkout-a").unwrap();
    let allowed_path = WorkspacePath::try_from("src/owned.rs".to_owned()).unwrap();
    let mut root_capabilities = Capabilities::default();
    root_capabilities.workspace_write = true;
    let root = approval_support::approved_root(RootDelegationInput {
        job_id: JobId::parse("job-a").unwrap(),
        agent_id: AgentId::parse(agent).unwrap(),
        invocation_id: InvocationId::parse(invocation).unwrap(),
        repository_id: RepositoryId::parse("repository-a").unwrap(),
        snapshot_id: SnapshotId::parse("snapshot-a").unwrap(),
        managed_checkout_id: Some(checkout),
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 3,
        write_paths: vec![allowed_path.clone()],
        capabilities: root_capabilities.clone(),
        database_profiles: Vec::new(),
    })
    .unwrap();
    root.narrow(
        ChildAuthorityRequest::new(
            ChildRole::Implementer,
            vec![
                allowed_path,
                WorkspacePath::try_from("src/not-root-owned.rs".to_owned()).unwrap(),
            ],
            root_capabilities,
            Vec::new(),
        )
        .unwrap(),
        PlatformSupport {
            workspace_edits: true,
            command_profile: Some(CommandCheckoutProfile::IsolatedPrivateCheckout),
            ..PlatformSupport::default()
        },
    )
    .unwrap()
}

pub fn request() -> AgentRequest {
    AgentRequest {
        prompt: "inspect the assigned source".to_owned(),
        profile_names: Vec::new(),
        model: MODEL.to_owned(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

pub fn limits(max_requests: u64) -> (AgentLimits, AnsweringLimits) {
    (
        AgentLimits {
            max_turns: Some(4),
            max_tool_calls: Some(64),
            max_continuations: Some(1),
            permit_candidate_writes: false,
            permit_workspace_writes: false,
            permit_external_effects: false,
            context_byte_budget: 64 * 1024,
        },
        AnsweringLimits {
            max_requests: Some(max_requests),
            max_tool_calls: Some(64),
            max_reported_tokens: Some(10_000),
        },
    )
}

pub fn spec(
    agent: &str,
    invocation: &str,
    request: AgentRequest,
    max_requests: u64,
) -> ChildExecutionSpec {
    let (agent_limits, answering_limits) = limits(max_requests);
    ChildExecutionSpec::new(
        authority_view(agent, invocation),
        request,
        agent_limits,
        answering_limits,
        None,
    )
    .unwrap()
}

#[path = "approval_support.rs"]
mod approval_support;
