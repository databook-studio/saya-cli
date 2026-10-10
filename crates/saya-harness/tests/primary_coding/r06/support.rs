use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use saya_agent::{AgentLimits, AnsweringLimits};
use saya_harness::coding::context::ChildProjectionOptions;
use saya_harness::coding::policy::{
    ChildAuthorityRequest, ChildRole, CommandCheckoutProfile, PlatformSupport, RootDelegationInput,
    RootDelegationScope,
};
use saya_harness::workspace::Workspace;
use saya_types::{
    AgentId, Capabilities, InvocationId, JobId, ManagedCheckoutId, RepositoryId, Sha256Digest,
    SnapshotId, WorkspacePath,
};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

pub(super) struct TempTree(PathBuf);

impl TempTree {
    pub(super) fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("saya-r06-{}-{id}", std::process::id()));
        match fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("stale test tree should be removable: {error}"),
        }
        fs::create_dir_all(&path).expect("temporary test tree should be created");
        Self(path)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }

    pub(super) fn workspace(&self) -> Workspace {
        Workspace::open(self.path()).expect("test workspace should open")
    }

    pub(super) fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("test file parent should be created");
        }
        fs::write(path, bytes).expect("test source should be written");
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn authority(tag: &str, path: &str) -> saya_harness::coding::policy::ChildAuthorityView {
    let checkout = ManagedCheckoutId::parse(&format!("checkout-{tag}")).unwrap();
    let write_path: WorkspacePath = path.to_owned().try_into().unwrap();
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    let root = RootDelegationScope::new(RootDelegationInput {
        job_id: JobId::parse(&format!("job-{tag}")).unwrap(),
        agent_id: AgentId::parse(&format!("agent-{tag}")).unwrap(),
        invocation_id: InvocationId::parse(&format!("invoke-{tag}")).unwrap(),
        repository_id: RepositoryId::parse("repo-r06").unwrap(),
        snapshot_id: SnapshotId::parse(&format!("snapshot-{tag}")).unwrap(),
        managed_checkout_id: Some(checkout),
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 4,
        write_paths: vec![write_path.clone()],
        capabilities: capabilities.clone(),
        database_profiles: Vec::new(),
    })
    .unwrap();
    let requested = ChildAuthorityRequest::new(
        ChildRole::Implementer,
        vec![write_path],
        capabilities,
        Vec::new(),
    )
    .unwrap();
    root.narrow(
        requested,
        PlatformSupport {
            workspace_edits: true,
            command_profile: Some(CommandCheckoutProfile::IsolatedPrivateCheckout),
            network_fetch: false,
            database_reads: false,
        },
    )
    .unwrap()
}

pub(super) fn options(model: &str) -> ChildProjectionOptions {
    ChildProjectionOptions {
        model: model.to_owned(),
        profile_names: Vec::new(),
        agent_limits: AgentLimits {
            max_turns: Some(2),
            max_tool_calls: Some(3),
            max_continuations: Some(0),
            permit_candidate_writes: false,
            permit_workspace_writes: false,
            permit_external_effects: false,
            context_byte_budget: 64 * 1024,
        },
        answering_limits: AnsweringLimits {
            max_requests: Some(1),
            max_tool_calls: Some(3),
            max_reported_tokens: Some(4_096),
        },
        request_preflight: None,
    }
}
