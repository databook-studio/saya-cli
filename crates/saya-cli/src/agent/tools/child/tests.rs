//! Tests for the child tool surface: one verified managed checkout, exact
//! admission, and no authority or path leakage. Authored, not executed:
//! saya-cli depends on DuckDB and is never built in this environment.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::{ToolDefinition, ToolError, ToolExecutor};
use saya_harness::coding::policy::{
    ChildAuthorityRequest, ChildAuthorityView, ChildRole, CommandCheckoutProfile, PlatformSupport,
    RootDelegationInput, RootDelegationScope,
};
use saya_harness::workspace::Workspace;
use saya_types::{
    AgentId, Capabilities, InvocationId, JobId, ManagedCheckoutId, RepositoryId, Sha256Digest,
    SnapshotId, WorkspacePath,
};

pub(super) fn checkout(id: &str) -> ManagedCheckoutId {
    ManagedCheckoutId::parse(id).unwrap()
}

fn path(value: &str) -> WorkspacePath {
    WorkspacePath::try_from(value.to_owned()).unwrap()
}

fn write_capabilities() -> Capabilities {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    capabilities
}

/// An Implementer narrowed to exactly `paths` with the workspace_write
/// capability on a platform that permits workspace edits.
pub(super) fn implementer_view(checkout: &ManagedCheckoutId, paths: &[&str]) -> ChildAuthorityView {
    RootDelegationScope::new(RootDelegationInput {
        job_id: JobId::parse("job").unwrap(),
        agent_id: AgentId::parse("agent").unwrap(),
        invocation_id: InvocationId::parse("invocation").unwrap(),
        repository_id: RepositoryId::parse("repository").unwrap(),
        snapshot_id: SnapshotId::parse("snapshot").unwrap(),
        managed_checkout_id: Some(checkout.clone()),
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 7,
        write_paths: paths.iter().map(|value| path(value)).collect(),
        capabilities: write_capabilities(),
        database_profiles: Vec::new(),
    })
    .unwrap()
    .narrow(
        ChildAuthorityRequest::new(
            ChildRole::Implementer,
            paths.iter().map(|value| path(value)).collect(),
            write_capabilities(),
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

fn advertises(definitions: &[ToolDefinition], name: &str) -> bool {
    definitions.iter().any(|tool| tool.name == name)
}

/// A sandbox workspace under the OS temp dir, removed on drop. The root sits
/// one level down (`outer/ws`) so a `../` escape has a real target outside.
pub(super) struct Sandbox {
    pub(super) outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    pub(super) fn new(label: &str) -> Self {
        let outer = std::env::temp_dir().join(format!("saya-child-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    pub(super) fn ws_root(&self) -> PathBuf {
        self.outer.join("ws")
    }

    /// A pair for `checkout_id` whose workspace root is this sandbox.
    pub(super) fn pair(&self, checkout_id: &ManagedCheckoutId) -> TrustedCheckoutPair {
        TrustedCheckoutPair::for_tests(checkout_id.clone(), Arc::new(self.ws.clone()))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

/// The read tools every child toolset advertises regardless of write permits.
const KNOWN_READS: [&str; 4] = ["workspace_read", "workspace_list", "glob", "grep"];

fn write_call(value: &str, content: &str) -> serde_json::Value {
    serde_json::json!({"path": value, "content": content})
}

// authored, not executed
#[tokio::test]
async fn write_definitions_absent_without_checkout_or_paths() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout, &[]);
    let sandbox = Sandbox::new("defs-absent");
    let pair = sandbox.pair(&checkout);

    let (definitions, _) = child_toolset(&view, &pair).unwrap();
    assert!(!advertises(&definitions, "workspace_write"));
    assert!(!advertises(&definitions, "workspace_edit"));
    assert!(advertises(&definitions, "workspace_read"));
}

// authored, not executed
#[tokio::test]
async fn definitions_and_executor_agree() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout, &["a/ok.txt"]);
    let sandbox = Sandbox::new("agree");
    let pair = sandbox.pair(&checkout);
    let (definitions, tools) = child_toolset(&view, &pair).unwrap();

    assert!(advertises(&definitions, "workspace_write"));
    assert!(advertises(&definitions, "workspace_edit"));
    for definition in &definitions {
        let name: &str = &definition.name;
        if KNOWN_READS.contains(&name) {
            continue;
        }
        let outcome = tools.execute(name, serde_json::json!({})).await;
        assert!(!matches!(outcome, Err(ToolError::UnsupportedTool)));
    }

    for name in ["bounded_sql_query", "run_program"] {
        let outcome = tools.execute(name, serde_json::json!({})).await;
        assert!(matches!(outcome, Err(ToolError::UnsupportedTool)));
    }

    let read_only_view = implementer_view(&checkout, &[]);
    let (_, read_only_tools) = child_toolset(&read_only_view, &pair).unwrap();
    let outcome = read_only_tools
        .execute("workspace_write", write_call("a/ok.txt", "x"))
        .await;
    assert!(matches!(outcome, Err(ToolError::UnsupportedTool)));
}

// authored, not executed
#[tokio::test]
async fn allowed_exact_path_writes_child_workspace_only() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout, &["a/ok.txt"]);
    let sandbox = Sandbox::new("exact-paths");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    let wrote = tools
        .execute("workspace_write", write_call("a/ok.txt", "x"))
        .await;
    assert!(wrote.is_ok());
    assert!(sandbox.ws_root().join("a/ok.txt").exists());

    let absolute = sandbox.outer.join("abs.txt");
    for refused in ["a/other.txt", "../escape.txt", absolute.to_str().unwrap()] {
        let outcome = tools
            .execute("workspace_write", write_call(refused, "x"))
            .await;
        assert!(matches!(outcome, Err(ToolError::WorkspaceWrite(_))));
    }
    let outcome = tools
        .execute("workspace_write", write_call("a/ok.txt/../other.txt", "x"))
        .await;
    assert!(matches!(outcome, Err(ToolError::WorkspaceWrite(_))));

    assert!(!sandbox.ws_root().join("a/other.txt").exists());
    assert!(!sandbox.outer.join("escape.txt").exists());
    assert!(!absolute.exists());
}

// authored, not executed
#[tokio::test]
async fn forged_checkout_id_refuses_at_build() {
    let forged = checkout("other-checkout");
    let view = implementer_view(&forged, &["a/ok.txt"]);
    let sandbox = Sandbox::new("forged");
    let pair = sandbox.pair(&checkout("checkout"));

    let built = child_toolset(&view, &pair);
    assert!(matches!(built, Err(ChildToolsError::CheckoutMismatch)));
}

// authored, not executed
#[tokio::test]
async fn read_tools_are_bound_to_child_root() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout, &["a/ok.txt"]);
    let sandbox = Sandbox::new("reads");
    fs::write(sandbox.ws_root().join("read.md"), "hello").expect("seed file must write");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    let read = tools
        .execute("workspace_read", serde_json::json!({"path": "read.md"}))
        .await;
    assert!(read.is_ok());
    let read = read.unwrap();
    assert!(read.to_string().contains("hello"));
    let escaped = tools
        .execute("workspace_read", serde_json::json!({"path": "../x"}))
        .await;
    assert!(escaped.is_err());
}

// authored, not executed
#[tokio::test]
async fn child_tools_debug_carries_no_path() {
    let checkout = checkout("checkout");
    let sandbox = Sandbox::new("debug");
    let pair = sandbox.pair(&checkout);

    let debug = format!("{pair:?}");
    assert!(debug.contains("checkout"));
    assert!(!debug.contains(sandbox.outer.to_string_lossy().as_ref()));
}
