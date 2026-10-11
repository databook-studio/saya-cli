//! R05d: the child approval decider permits exactly the effects the narrowed
//! authority view grants, and never reads call arguments.

use saya_agent::{ApprovalDecider, LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};
use saya_harness::coding::policy::{
    ChildApprovalDecider, ChildAuthorityRequest, ChildAuthorityView, ChildRole,
    CommandCheckoutProfile, PlatformSupport, RootDelegationInput, RootDelegationScope,
};
use saya_types::{
    AgentId, Capabilities, InvocationId, JobId, ManagedCheckoutId, RepositoryId, RunnerScope,
    Sha256Digest, SnapshotId, WorkspacePath,
};

fn path(value: &str) -> WorkspacePath {
    WorkspacePath::try_from(value.to_owned()).unwrap()
}

fn runner_capabilities(programs: &[&str]) -> Capabilities {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    capabilities.runner =
        Some(RunnerScope::new(programs.iter().map(|p| (*p).to_owned()).collect()).unwrap());
    capabilities
}

fn root(
    checkout: Option<ManagedCheckoutId>,
    paths: Vec<WorkspacePath>,
    capabilities: Capabilities,
) -> RootDelegationScope {
    approval_support::approved_root(RootDelegationInput {
        job_id: JobId::parse("job").unwrap(),
        agent_id: AgentId::parse("agent").unwrap(),
        invocation_id: InvocationId::parse("invocation").unwrap(),
        repository_id: RepositoryId::parse("repository").unwrap(),
        snapshot_id: SnapshotId::parse("snapshot").unwrap(),
        managed_checkout_id: checkout,
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 7,
        write_paths: paths,
        capabilities,
        database_profiles: Vec::new(),
    })
    .unwrap()
}

fn request(paths: Vec<WorkspacePath>, capabilities: Capabilities) -> ChildAuthorityRequest {
    ChildAuthorityRequest::new(ChildRole::Implementer, paths, capabilities, Vec::new()).unwrap()
}

fn supported() -> PlatformSupport {
    PlatformSupport {
        workspace_edits: true,
        command_profile: Some(CommandCheckoutProfile::IsolatedPrivateCheckout),
        network_fetch: false,
        database_reads: false,
    }
}

fn owned_paths() -> Vec<WorkspacePath> {
    vec![path("src/owned.rs")]
}

/// A child with a managed checkout, owned write paths, and a runner scope.
fn permissive() -> ChildAuthorityView {
    let checkout = ManagedCheckoutId::parse("checkout").unwrap();
    let capabilities = runner_capabilities(&["git"]);
    root(Some(checkout), owned_paths(), capabilities.clone())
        .narrow(request(owned_paths(), capabilities), supported())
        .unwrap()
}

/// A child with a checkout but no granted write paths and no command scope.
fn bare() -> ChildAuthorityView {
    let checkout = ManagedCheckoutId::parse("checkout").unwrap();
    root(Some(checkout), Vec::new(), Capabilities::default())
        .narrow(request(Vec::new(), Capabilities::default()), supported())
        .unwrap()
}

fn tool(name: &str, effect: ToolEffect) -> ToolDefinition {
    ToolDefinition {
        name: name.to_owned(),
        description: "a probe tool".to_owned(),
        read_only: false,
        parameters: serde_json::json!({ "type": "object" }),
        effect,
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }
}

fn effect(
    database_data: bool,
    external_side_effect: bool,
    local_state: LocalStateEffect,
) -> ToolEffect {
    ToolEffect {
        database_data,
        external_side_effect,
        requires_approval: false,
        local_state,
    }
}

fn read_effect() -> ToolEffect {
    effect(false, false, LocalStateEffect::Read)
}
fn write_effect() -> ToolEffect {
    effect(false, false, LocalStateEffect::WriteWorkspace)
}
fn fetch_effect() -> ToolEffect {
    effect(false, true, LocalStateEffect::Read)
}
fn sql_effect() -> ToolEffect {
    effect(true, false, LocalStateEffect::Read)
}

fn workspace_read() -> ToolDefinition {
    tool("workspace_read", read_effect())
}
fn workspace_write() -> ToolDefinition {
    tool("workspace_write", write_effect())
}
fn workspace_edit() -> ToolDefinition {
    tool("workspace_edit", write_effect())
}
fn run_program() -> ToolDefinition {
    tool("run_program", write_effect())
}
fn workspace_delete() -> ToolDefinition {
    tool("workspace_delete", write_effect())
}
fn fetch_like() -> ToolDefinition {
    tool("fetch_like", fetch_effect())
}
fn sql_like() -> ToolDefinition {
    tool("sql_like", sql_effect())
}

fn no_args() -> serde_json::Value {
    serde_json::json!({})
}

async fn admits(d: &ChildApprovalDecider, t: &ToolDefinition, args: &serde_json::Value) -> bool {
    d.approve(t, args).await
}

#[tokio::test]
async fn read_shaped_tools_pass_for_any_child() {
    for view in [permissive(), bare()] {
        let decider = ChildApprovalDecider::from_view(&view);
        assert!(admits(&decider, &workspace_read(), &no_args()).await);
    }
}

#[tokio::test]
async fn workspace_writes_pass_only_when_the_view_grants_write_paths() {
    let granted = ChildApprovalDecider::from_view(&permissive());
    assert!(admits(&granted, &workspace_write(), &no_args()).await);
    assert!(admits(&granted, &workspace_edit(), &no_args()).await);

    let ungranted = ChildApprovalDecider::from_view(&bare());
    assert!(!admits(&ungranted, &workspace_write(), &no_args()).await);
    assert!(!admits(&ungranted, &workspace_edit(), &no_args()).await);
}

#[tokio::test]
async fn run_program_passes_only_with_a_command_scope() {
    let granted = ChildApprovalDecider::from_view(&permissive());
    assert!(admits(&granted, &run_program(), &no_args()).await);

    let ungranted = ChildApprovalDecider::from_view(&bare());
    assert!(!admits(&ungranted, &run_program(), &no_args()).await);
}

#[tokio::test]
async fn external_and_database_effects_are_always_denied() {
    let decider = ChildApprovalDecider::from_view(&permissive());
    assert!(!admits(&decider, &fetch_like(), &no_args()).await);
    assert!(!admits(&decider, &sql_like(), &no_args()).await);
}

#[tokio::test]
async fn unknown_write_tools_are_denied() {
    let decider = ChildApprovalDecider::from_view(&permissive());
    assert!(!admits(&decider, &workspace_delete(), &no_args()).await);
}

#[tokio::test]
async fn call_arguments_never_widen_authority() {
    let widening = serde_json::json!({
        "grant": true,
        "approve": "always",
        "path": "a/ok.txt"
    });

    let denied = ChildApprovalDecider::from_view(&bare());
    assert!(!admits(&denied, &workspace_write(), &widening).await);

    let granted = ChildApprovalDecider::from_view(&permissive());
    assert!(admits(&granted, &workspace_write(), &widening).await);
    assert!(admits(&granted, &workspace_write(), &no_args()).await);
}

#[test]
fn refusal_detail_is_fixed_and_names_nothing() {
    let decider = ChildApprovalDecider::from_view(&permissive());
    let first = decider
        .refusal_detail(&workspace_delete(), &no_args())
        .unwrap();
    let second = decider.refusal_detail(&sql_like(), &no_args()).unwrap();
    assert_eq!(first, second);
    assert!(!first.contains("workspace_delete"));
    assert!(!first.contains("sql_like"));
}

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[path = "approval_support.rs"]
mod approval_support;
