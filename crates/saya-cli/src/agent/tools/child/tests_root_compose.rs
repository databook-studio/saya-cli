//! Direct-mode root workspace composition tests: the root's workspace toolset
//! is composed over the agent-less working checkout W from the SAME authority
//! path a child uses, so the parent and its children advertise the same
//! workspace tools. Authored, not executed in this environment; the tests
//! exercise the real
//! verify→confine→approve→narrow→build flow against a real git checkout.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, CancellationToken, LocalStateEffect, ToolDefinition, ToolExecutor,
};
use saya_harness::coding::policy::{ChildAuthorityRequest, ChildRole, RootDelegationInput};
use saya_types::{AgentId, Capabilities, InvocationId, Sha256Digest};

use super::super::child_toolset;
use super::super::tests::{Sandbox, checkout, implementer_view};
use super::{CheckoutProof, RootComposeRequest, RootWorkspace, compose_root_workspace};

use super::super::tests_root_compose_fixture as fixture;

/// Fresh host consent for the root scope: approves only `coding_root_scope`,
/// mirroring `approval_test_support::FixtureHost` (which stays private to its
/// own module). The root delegation is approved once, here, with this decider.
struct RootConsent;
#[async_trait]
impl ApprovalDecider for RootConsent {
    async fn approve(&self, tool: &ToolDefinition, _: &serde_json::Value) -> bool {
        tool.name == "coding_root_scope"
    }
}

fn write_capabilities() -> Capabilities {
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    capabilities
}

/// Composes the root's workspace toolset over W with the write capability only
/// (no command scope, no coding, no databases), so the advertised toolset is
/// exactly the workspace tools a write-capable child gets.
async fn compose_root(fixture: &fixture::Fixture) -> RootWorkspace {
    compose_root_with(fixture, Vec::new(), None).await
}

/// The same composition with a selected database profile: the root's database
/// surface is the job's, over the profiles its own consent selected.
async fn compose_root_with(
    fixture: &fixture::Fixture,
    database_profiles: Vec<saya_types::ProfileIdentity>,
    databases: Option<crate::commands::coding::database::CodingDatabaseTools>,
) -> RootWorkspace {
    let cancellation = CancellationToken::new();
    let capabilities = write_capabilities();
    let root = RootDelegationInput {
        job_id: fixture.job.clone(),
        agent_id: AgentId::parse("coordinator").unwrap(),
        invocation_id: InvocationId::parse("invocation").unwrap(),
        repository_id: fixture.repository_id.clone(),
        snapshot_id: fixture.snapshot_id.clone(),
        managed_checkout_id: Some(fixture.checkout.clone()),
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 7,
        capabilities: capabilities.clone(),
        database_profiles: database_profiles.clone(),
    };
    let authority =
        ChildAuthorityRequest::new(ChildRole::Implementer, capabilities, database_profiles)
            .unwrap();
    let consent = RootConsent;
    let request = RootComposeRequest {
        proof: CheckoutProof::Managed(fixture.ownership()),
        verified: None,
        cancellation: &cancellation,
        write_scope: None,
        root,
        decider: &consent,
        authority,
        runtime: &fixture.runtime,
        coding: None,
        databases,
        tools: saya_config::ResolvedTools::default(),
        spool_dir: None,
    };
    compose_root_workspace(request).await.unwrap()
}

/// The advertised shape of one toolset: each definition's name, its
/// local-state effect, and whether the loop must consult the child decider
/// before running it. The root and a child for the same paths must advertise
/// the same shape.
fn shape(definitions: &[ToolDefinition]) -> Vec<(String, LocalStateEffect, bool)> {
    definitions
        .iter()
        .map(|definition| {
            (
                definition.name.clone(),
                definition.effect.local_state,
                definition.effect.requires_approval,
            )
        })
        .collect()
}

// authored, not executed
#[tokio::test]
async fn the_root_workspace_toolset_matches_a_child_toolset() {
    let fixture = fixture::Fixture::new("match", &["tracked.txt"]).await;
    let root = compose_root(&fixture).await;

    // A write-capable child toolset built directly from the child authority
    // path with no provider, coding, or databases — exactly what
    // `build_toolset` produces for the root when `coding`/`databases` are
    // `None`.
    let child_checkout = checkout("child-checkout");
    let child_view = implementer_view(&child_checkout);
    let sandbox = Sandbox::new("root-match");
    let pair = sandbox.pair(&child_checkout);
    let (child_definitions, _) = child_toolset(&child_view, &pair).unwrap();

    assert_eq!(shape(&root.definitions), shape(&child_definitions));
    assert!(
        root.definitions
            .iter()
            .any(|definition| definition.name == "write")
    );
    assert!(
        root.definitions
            .iter()
            .any(|definition| definition.name == "read")
    );
}

/// The workspace toolset does not run the sandbox probe: its file tools admit
/// every write in-process against the verified checkout, so a checkout root
/// outside the host profile language's safe text class still composes. Only a
/// spawned command needs a proven sandbox, and the shell that spawns one is
/// absent — not an error — where this host cannot prove it.
#[tokio::test]
async fn composition_does_not_require_a_sandbox_provable_root() {
    let fixture = fixture::Fixture::with_directory_name(
        "probe-deferred",
        &["tracked.txt"],
        "saya-root-compose-probe (deferred)",
    )
    .await;
    let root = compose_root(&fixture).await;
    assert!(
        root.definitions
            .iter()
            .any(|definition| definition.name == "write"),
        "the workspace tools compose without a sandbox proof"
    );
}

/// The direct root advertises the standard read-only database tools for the
/// profile its own consent selected, and never a `coding_*` alias.
#[tokio::test]
async fn the_root_advertises_the_standard_database_tools() {
    let fixture = fixture::Fixture::new("root-databases", &["tracked.txt"]).await;
    let profile = saya_types::DatabaseProfile::Sqlite {
        path: fixture
            .checkout_path
            .join("root.sqlite3")
            .to_str()
            .unwrap()
            .to_owned(),
        read_only: true,
    };
    let identity =
        super::super::tests::profile_identity("main", &profile, &fixture.runtime.cache_scope);
    // The data-sharing gate is the run's resolved policy; this test is about
    // which standard tools the root advertises, so it is on.
    let mut runtime = fixture.runtime.clone();
    runtime.resolved.ai.allow_data_sharing = true;
    let root = compose_root_with(
        &fixture,
        vec![identity.clone()],
        Some(crate::commands::coding::database::CodingDatabaseTools::new(
            runtime,
            fixture.store.clone(),
            std::collections::BTreeMap::from([("main".to_owned(), identity)]),
            false,
        )),
    )
    .await;

    let mut database_tools: Vec<&str> = root
        .definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .filter(|name| {
            !crate::agent::tools::workspace::is_tool_name(name)
                && *name != "checklist"
                && *name != "helpers"
        })
        .collect();
    database_tools.sort_unstable();
    assert_eq!(
        database_tools,
        ["bounded_sql_query", "schema_discovery"],
        "the root advertises exactly the standard read-only database tools"
    );
}

// authored, not executed
#[tokio::test]
async fn a_root_write_anywhere_in_the_working_checkout_is_admitted() {
    let fixture = fixture::Fixture::new("deny", &["tracked.txt"]).await;
    let root = compose_root(&fixture).await;

    let write = root
        .definitions
        .iter()
        .find(|definition| definition.name == "write")
        .expect("write is advertised")
        .clone();

    // There is no path ceiling: the root writes any path in W, so a new file
    // is admitted and the executor performs it.
    let anywhere = serde_json::json!({"path": "outside.txt", "content": "x"});
    assert!(root.approval.approve(&write, &anywhere).await);
    let outcome = root.tools.execute("write", anywhere).await;
    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(fixture.checkout_path.join("outside.txt").exists());

    // The bound that survives is the workspace's own containment: a path the
    // checkout cannot hold is refused before execution, by the decider and by
    // the executor's admission alike.
    let escaping = serde_json::json!({"path": "../escape.txt", "content": "x"});
    assert!(!root.approval.approve(&write, &escaping).await);
    assert!(root.tools.execute("write", escaping).await.is_err());
}

// authored, not executed
#[tokio::test]
async fn a_root_write_of_a_new_file_in_the_working_checkout_is_admitted() {
    let fixture = fixture::Fixture::new("admit", &["tracked.txt"]).await;
    // A path W does not already contain, so `write` creates it (overwriting
    // an existing file would require a fresh-read digest).
    let root = compose_root(&fixture).await;

    let write = root
        .definitions
        .iter()
        .find(|definition| definition.name == "write")
        .expect("write is advertised")
        .clone();
    let inside = serde_json::json!({"path": "root_wrote.txt", "content": "root wrote"});

    // Admitted by the decider: the root writes its whole working checkout.
    let admitted = root.approval.approve(&write, &inside).await;
    assert!(admitted);

    // The executor writes the file inside W.
    let outcome = root.tools.execute("write", inside).await;
    assert!(
        outcome.is_ok(),
        "executor error: {:?}",
        outcome.as_ref().err()
    );
    let written = std::fs::read_to_string(fixture.checkout_path.join("root_wrote.txt")).unwrap();
    assert_eq!(written, "root wrote");
}

// authored, not executed
#[tokio::test]
async fn coordinator_is_the_root_id_and_is_rejected_as_a_child_id() {
    let fixture = fixture::Fixture::new("coordinator-id", &["tracked.txt"]).await;
    // `compose_root_workspace` narrows the root delegation (agent_id
    // "coordinator") and validates it against the agent-less checkout W;
    // succeeding proves the in-memory authority view carries "coordinator"
    // while W stays agent-less (its agent id is None).
    let _ = compose_root(&fixture).await;

    // "coordinator" is reserved for the root and never admitted as a child id:
    // `validate_delegate_tasks` refuses it. This change does not alter that
    // behaviour; the assertion pins it.
    let task = crate::commands::coding::CodingTask {
        agent_id: AgentId::parse("coordinator").unwrap(),
        task: "coordinator must never be a child id".into(),
        subagent_type: "general".into(),
        write_paths: Vec::new(),
        dependencies: Vec::new(),
    };
    let result = fixture.delegate_job.validate_delegate_tasks(&[task]);
    assert!(
        matches!(
            result,
            Err(crate::commands::coding::CodingError::InvalidRequest)
        ),
        "coordinator must be rejected as a child id, got {result:?}"
    );
}
