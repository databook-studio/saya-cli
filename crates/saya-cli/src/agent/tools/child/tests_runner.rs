//! Tests for the child `run_program` tool: the runner arrives as an
//! already-confined `RunProgram`, admission goes through the view scope
//! before delegation, and without a runner the tool is absent. Authored, not
//! executed: saya-cli depends on DuckDB and is never built in this
//! environment.

use std::{fs, path::Path, sync::Arc, time::Duration};

use super::tests::{Sandbox, checkout, implementer_view};
use super::{ChildToolsError, child_toolset, child_toolset_with_runner};
use saya_agent::{ToolDefinition, ToolError, ToolExecutor};
use saya_harness::coding::policy::{
    ChildAuthorityRequest, ChildAuthorityView, ChildRole, CommandCheckoutProfile, PlatformSupport,
    RootDelegationInput,
};
use saya_harness::runner::sandbox::{RunSandbox, SandboxProvision};
use saya_harness::runner::{RunProgram, StaticCredentialSource};
use saya_types::{
    AgentId, Capabilities, InvocationId, JobId, ManagedCheckoutId, RepositoryId, RunnerScope,
    Sha256Digest, SnapshotId, WorkspacePath,
};

fn path(value: &str) -> WorkspacePath {
    WorkspacePath::try_from(value.to_owned()).unwrap()
}

/// An Implementer narrowed to exactly `["a/ok.txt"]` whose capabilities grant
/// the runner exactly `programs`: both the root input and the request
/// capabilities carry the same runner scope, on a platform with an isolated
/// private command checkout.
pub(super) fn runner_view(checkout: &ManagedCheckoutId, programs: &[&str]) -> ChildAuthorityView {
    runner_view_for_paths(checkout, programs, &["a/ok.txt"])
}

pub(super) fn command_only_view(
    checkout: &ManagedCheckoutId,
    programs: &[&str],
) -> ChildAuthorityView {
    runner_view_for_paths(checkout, programs, &[])
}

fn runner_view_for_paths(
    checkout: &ManagedCheckoutId,
    programs: &[&str],
    paths: &[&str],
) -> ChildAuthorityView {
    let scope = || RunnerScope::new(programs.iter().map(|p| p.to_string()).collect()).unwrap();
    let write_paths = || paths.iter().map(|value| path(value)).collect::<Vec<_>>();
    let mut root_capabilities = Capabilities::default();
    root_capabilities.workspace_write = true;
    root_capabilities.runner = Some(scope());
    let mut request_capabilities = root_capabilities.clone();
    request_capabilities.runner = Some(scope());
    super::approval_test_support::approved_root(RootDelegationInput {
        job_id: JobId::parse("job").unwrap(),
        agent_id: AgentId::parse("agent").unwrap(),
        invocation_id: InvocationId::parse("invocation").unwrap(),
        repository_id: RepositoryId::parse("repository").unwrap(),
        snapshot_id: SnapshotId::parse("snapshot").unwrap(),
        managed_checkout_id: Some(checkout.clone()),
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 7,
        write_paths: write_paths(),
        capabilities: root_capabilities,
        database_profiles: Vec::new(),
    })
    .unwrap()
    .narrow(
        ChildAuthorityRequest::new(
            ChildRole::Implementer,
            write_paths(),
            request_capabilities,
            Vec::new(),
        )
        .unwrap(),
        PlatformSupport {
            workspace_edits: true,
            command_profile: Some(CommandCheckoutProfile::IsolatedPrivateCheckout),
            network_fetch: true,
            database_reads: true,
        },
    )
    .unwrap()
}

/// A real runner proven on this host against the sandbox root, recording
/// outside it. `None` when the host probe refuses the sandbox: an accepted
/// skip.
fn real_runner(root: &Path, records: &Path) -> Option<Arc<RunProgram>> {
    let sandbox = RunSandbox::new([root.to_path_buf()], std::iter::empty()).unwrap();
    match sandbox.prepare(Path::new("/bin")).unwrap() {
        SandboxProvision::Proven { spawn, .. } => {
            fs::create_dir_all(records).expect("records directory must create");
            Some(Arc::new(
                RunProgram::for_step(
                    spawn,
                    Some(RunnerScope::new(vec!["pwd".into(), "ls".into()]).unwrap()),
                    None,
                    Duration::from_secs(10),
                    Arc::new(StaticCredentialSource::new([])),
                )
                .with_record_dir(records.to_path_buf()),
            ))
        }
        SandboxProvision::Refused(_) => None,
    }
}

fn advertises(definitions: &[ToolDefinition], name: &str) -> bool {
    definitions.iter().any(|tool| tool.name == name)
}

fn count(definitions: &[ToolDefinition], name: &str) -> usize {
    definitions.iter().filter(|tool| tool.name == name).count()
}

/// The read tools every child toolset advertises regardless of write permits.
const KNOWN_READS: [&str; 4] = ["workspace_read", "workspace_list", "glob", "grep"];

// authored, not executed
#[tokio::test]
async fn run_program_is_absent_without_a_runner() {
    let checkout = checkout("checkout");
    let view = runner_view(&checkout, &["pwd"]);
    let sandbox = Sandbox::new("runner-absent");
    let pair = sandbox.pair(&checkout);

    let (definitions, tools) = child_toolset(&view, &pair).ok().unwrap();
    assert!(!advertises(&definitions, "run_program"));
    let outcome = tools
        .execute("run_program", serde_json::json!({"program": "pwd"}))
        .await;
    assert!(matches!(outcome, Err(ToolError::UnsupportedTool)));
}

// authored, not executed
#[tokio::test]
async fn runner_requires_a_view_scope() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout, &["a/ok.txt"]);
    let sandbox = Sandbox::new("runner-scope");
    let pair = sandbox.pair(&checkout);
    let Some(runner) = real_runner(&sandbox.ws_root(), &sandbox.outer.join("records")) else {
        return;
    };

    let built = child_toolset_with_runner(&view, &pair, runner);
    assert!(matches!(built, Err(ChildToolsError::RunnerNotAdmitted)));
}

// authored, not executed
#[tokio::test]
async fn run_program_admits_only_view_programs_before_delegation() {
    let checkout = checkout("checkout");
    let view = runner_view(&checkout, &["pwd"]);
    let sandbox = Sandbox::new("runner-admission");
    let pair = sandbox.pair(&checkout);
    let Some(runner) = real_runner(&sandbox.ws_root(), &sandbox.outer.join("records")) else {
        return;
    };

    let (_, tools) = child_toolset_with_runner(&view, &pair, runner)
        .ok()
        .unwrap();

    let refused = tools
        .execute("run_program", serde_json::json!({"program": "ls"}))
        .await;
    assert!(matches!(refused, Err(ToolError::Runner(_))));

    let admitted = tools
        .execute("run_program", serde_json::json!({"program": "pwd"}))
        .await;
    assert!(admitted.is_ok());

    let command = tools
        .execute("run_command", serde_json::json!({"program": "pwd"}))
        .await;
    assert!(matches!(command, Err(ToolError::UnsupportedTool)));
}

// authored, not executed
#[tokio::test]
async fn definitions_and_executor_agree_with_runner() {
    let checkout = checkout("checkout");
    let view = runner_view(&checkout, &["pwd"]);
    let sandbox = Sandbox::new("runner-agree");
    let pair = sandbox.pair(&checkout);
    let Some(runner) = real_runner(&sandbox.ws_root(), &sandbox.outer.join("records")) else {
        return;
    };

    let (definitions, tools) = child_toolset_with_runner(&view, &pair, runner)
        .ok()
        .unwrap();
    assert_eq!(count(&definitions, "run_program"), 1);
    for definition in &definitions {
        let name: &str = &definition.name;
        if KNOWN_READS.contains(&name) {
            continue;
        }
        let outcome = tools.execute(name, serde_json::json!({})).await;
        assert!(!matches!(outcome, Err(ToolError::UnsupportedTool)));
    }

    let without = child_toolset(&view, &pair);
    assert!(without.is_ok());
    let (without_definitions, without_tools) = without.ok().unwrap();
    assert!(!advertises(&without_definitions, "run_program"));
    let outcome = without_tools
        .execute("run_program", serde_json::json!({"program": "pwd"}))
        .await;
    assert!(matches!(outcome, Err(ToolError::UnsupportedTool)));
}
