//! Tests for the child tool surface: one verified managed checkout, exact
//! admission, and no authority or path leakage. Authored, not executed in
//! this environment.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::{ToolDefinition, ToolError, ToolExecutor};
use saya_harness::coding::policy::{
    ChildAuthorityRequest, ChildAuthorityView, ChildRole, PlatformSupport, RootDelegationInput,
};
use saya_harness::workspace::Workspace;
use saya_types::{
    AgentId, Capabilities, DatabaseProfile, InvocationId, JobId, ManagedCheckoutId,
    ProfileIdentity, RepositoryId, Sha256Digest, SnapshotId,
};

pub(super) fn checkout(id: &str) -> ManagedCheckoutId {
    ManagedCheckoutId::parse(id).unwrap()
}

fn write_capabilities() -> Capabilities {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    capabilities
}

/// A write-capable Implementer over `checkout` on a platform that permits
/// workspace edits: it may write any path in the shared working checkout.
pub(crate) fn implementer_view(checkout: &ManagedCheckoutId) -> ChildAuthorityView {
    view(checkout, write_capabilities())
}

/// An Implementer over `checkout` with **no** write capability: the read-only
/// shape an `explore` helper holds over the same shared working checkout.
pub(crate) fn read_only_view(checkout: &ManagedCheckoutId) -> ChildAuthorityView {
    view(checkout, Capabilities::default())
}

/// A write-capable Implementer over `checkout` that admits exactly `profiles`:
/// the shape a job with selected database profiles narrows onto its root and
/// its helpers.
pub(crate) fn database_view(
    checkout: &ManagedCheckoutId,
    profiles: Vec<ProfileIdentity>,
) -> ChildAuthorityView {
    view_with_profiles(checkout, write_capabilities(), profiles)
}

/// The identity a profile resolves to under `cache_scope`: the same derivation
/// the job's consent and the database surface use.
pub(crate) fn profile_identity(
    name: &str,
    profile: &DatabaseProfile,
    cache_scope: &std::path::Path,
) -> ProfileIdentity {
    crate::profile_identity::profile_identity(name, profile, cache_scope)
}

fn view(checkout: &ManagedCheckoutId, capabilities: Capabilities) -> ChildAuthorityView {
    view_with_profiles(checkout, capabilities, Vec::new())
}

fn view_with_profiles(
    checkout: &ManagedCheckoutId,
    capabilities: Capabilities,
    database_profiles: Vec<ProfileIdentity>,
) -> ChildAuthorityView {
    super::approval_test_support::approved_root(RootDelegationInput {
        job_id: JobId::parse("job").unwrap(),
        agent_id: AgentId::parse("agent").unwrap(),
        invocation_id: InvocationId::parse("invocation").unwrap(),
        repository_id: RepositoryId::parse("repository").unwrap(),
        snapshot_id: SnapshotId::parse("snapshot").unwrap(),
        managed_checkout_id: Some(checkout.clone()),
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 7,
        capabilities: capabilities.clone(),
        database_profiles: database_profiles.clone(),
    })
    .unwrap()
    .narrow(
        ChildAuthorityRequest::new(ChildRole::Implementer, capabilities, database_profiles)
            .unwrap(),
        PlatformSupport {
            workspace_edits: true,
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
const KNOWN_READS: [&str; 4] = ["read", "list", "glob", "grep"];

fn write_call(value: &str, content: &str) -> serde_json::Value {
    serde_json::json!({"path": value, "content": content})
}

// authored, not executed
#[tokio::test]
async fn write_definitions_absent_without_the_write_capability() {
    let checkout = checkout("checkout");
    let view = read_only_view(&checkout);
    let sandbox = Sandbox::new("defs-absent");
    let pair = sandbox.pair(&checkout);

    let (definitions, _) = child_toolset(&view, &pair).unwrap();
    assert!(!advertises(&definitions, "write"));
    assert!(!advertises(&definitions, "edit"));
    assert!(!advertises(&definitions, "shell"));
    assert!(advertises(&definitions, "checklist"));
    assert!(advertises(&definitions, "read"));
}

// authored, not executed
#[tokio::test]
async fn definitions_and_executor_agree() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("agree");
    let pair = sandbox.pair(&checkout);
    let (definitions, tools) = child_toolset(&view, &pair).unwrap();

    assert!(advertises(&definitions, "write"));
    assert!(advertises(&definitions, "edit"));
    assert!(!advertises(&definitions, "shell"));
    assert!(!advertises(&definitions, "bash"));
    assert!(advertises(&definitions, "checklist"));
    assert!(!advertises(&definitions, "helpers"));
    for definition in &definitions {
        let name: &str = &definition.name;
        if KNOWN_READS.contains(&name) {
            continue;
        }
        let outcome = tools.execute(name, serde_json::json!({})).await;
        assert!(!matches!(outcome, Err(ToolError::UnsupportedTool)));
    }

    for name in ["bounded_sql_query", "run_program", "helpers"] {
        let outcome = tools.execute(name, serde_json::json!({})).await;
        assert!(matches!(outcome, Err(ToolError::UnsupportedTool)));
    }

    assert!(tools.turn_context_note().is_none());
    tools
        .execute(
            "checklist",
            serde_json::json!({
                "tasks": [{"title": "parse records", "status": "in_progress"}]
            }),
        )
        .await
        .expect("checklist stores");
    let note = tools
        .turn_context_note()
        .expect("the list rides the next turn");
    assert!(note.contains("source: session-tasks"), "{note}");
    assert!(note.contains("parse records"), "{note}");
    assert!(
        tools.turn_context_note().is_none(),
        "a turn whose list did not change carries no note: the model already has the state"
    );

    let no_write_view = read_only_view(&checkout);
    let (_, read_only_tools) = child_toolset(&no_write_view, &pair).unwrap();
    let outcome = read_only_tools
        .execute("write", write_call("a/ok.txt", "x"))
        .await;
    assert!(matches!(
        outcome,
        Err(ToolError::NoEffect(inner))
            if matches!(inner.as_ref(), ToolError::UnsupportedTool)
    ));
}

/// `child_toolset` alone advertises no shell: the helper's shell is the
/// composition root's to attach (`ChildTools::attach_shell`), and until it
/// does the name stays unadvertised and undispatched. The attached case is
/// `tests_shell.rs`'s.
// authored, not executed
#[tokio::test]
async fn a_child_toolset_advertises_no_shell_until_one_is_attached() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("no-shell");
    let pair = sandbox.pair(&checkout);
    let (definitions, tools) = child_toolset(&view, &pair).unwrap();

    assert!(!advertises(&definitions, "shell"));
    assert!(!advertises(&definitions, "bash"));

    for name in ["shell", "bash"] {
        let outcome = tools
            .execute(name, serde_json::json!({"command": "echo hi"}))
            .await;
        assert!(
            matches!(outcome, Err(ToolError::UnsupportedTool)),
            "{name} must be unsupported: {outcome:?}"
        );
    }
}

/// There is no per-child path ceiling: a write-capable child writes any path
/// in the shared working checkout — no per-child path set to stay inside —
/// and the workspace's own containment is the bound that applies.
// authored, not executed
#[tokio::test]
async fn a_write_capable_child_writes_anywhere_inside_its_workspace() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("exact-paths");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    for path in ["a/ok.txt", "a/other.txt", "anywhere/deep/inside.txt"] {
        let wrote = tools.execute("write", write_call(path, "x")).await;
        assert!(wrote.is_ok(), "{path}: {wrote:?}");
        assert!(sandbox.ws_root().join(path).exists());
    }

    let absolute = sandbox.outer.join("abs.txt");
    for refused in ["../escape.txt", absolute.to_str().unwrap()] {
        let outcome = tools.execute("write", write_call(refused, "x")).await;
        assert!(matches!(
            outcome,
            Err(ToolError::NoEffect(inner))
                if matches!(inner.as_ref(), ToolError::WorkspaceWrite(_))
        ));
    }
    let outcome = tools
        .execute("write", write_call("a/ok.txt/../other.txt", "x"))
        .await;
    assert!(matches!(
        outcome,
        Err(ToolError::NoEffect(inner))
            if matches!(inner.as_ref(), ToolError::WorkspaceWrite(_))
    ));

    assert!(!sandbox.outer.join("escape.txt").exists());
    assert!(!absolute.exists());
}

// authored, not executed
#[tokio::test]
async fn forged_checkout_id_refuses_at_build() {
    let forged = checkout("other-checkout");
    let view = implementer_view(&forged);
    let sandbox = Sandbox::new("forged");
    let pair = sandbox.pair(&checkout("checkout"));

    let built = child_toolset(&view, &pair);
    assert!(matches!(built, Err(ChildToolsError::CheckoutMismatch)));
}

// authored, not executed
#[tokio::test]
async fn read_tools_are_bound_to_child_root() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("reads");
    fs::write(sandbox.ws_root().join("read.md"), "hello").expect("seed file must write");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    let read = tools
        .execute("read", serde_json::json!({"path": "read.md"}))
        .await;
    assert!(read.is_ok());
    let read = read.unwrap();
    assert!(read.to_string().contains("hello"));
    let escaped = tools
        .execute("read", serde_json::json!({"path": "../x"}))
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
