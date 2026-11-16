//! Root tool surface for a job with a working checkout: the child workspace
//! tools (over the agent-less working checkout W) plus the coordinator's
//! optional delegation tools, behind the root's write decider.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, CancellationToken, ToolDefinition, ToolError, ToolExecutor};
use saya_harness::coding::policy::{ChildAuthorityRequest, ChildRole, RootDelegationInput};
use saya_types::{AgentId, Capabilities, InvocationId, Sha256Digest};

use super::direct::RootTools;
use super::direct_tools::DirectCodingTools;
use super::parent::CodingTools;
use crate::agent::tools::child::{
    CheckoutProof, RootComposeRequest, RootWorkspace, compose_root_workspace,
};
use crate::commands::coding::database::CodingDatabaseTools;

use crate::agent::tools::child::tests_root_compose_fixture as fixture;

/// Fresh host consent for the root scope: approves only `coding_root_scope`.
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
/// (no command scope, no coding, no databases), under a stated `[tools] allow`
/// list. The list rides the compose request into the child factory, so it is
/// the helper's set the test reads back — the job's, minus `helpers`.
async fn compose_root_with(
    fixture: &fixture::Fixture,
    tools: saya_config::ResolvedTools,
) -> RootWorkspace {
    compose_root_data(fixture, tools, None).await
}

/// The same composition carrying the job's data surface, exactly as the job
/// hands it to `compose_direct_workspace`: the workspace's own toolset is what
/// serves the database tools and the ask on this root, so the surface rides the
/// compose request rather than the coordinator. Shared with the ask tests.
pub(super) async fn compose_root_data(
    fixture: &fixture::Fixture,
    tools: saya_config::ResolvedTools,
    databases: Option<CodingDatabaseTools>,
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
        database_profiles: Vec::new(),
    };
    let authority =
        ChildAuthorityRequest::new(ChildRole::Implementer, capabilities, Vec::new()).unwrap();
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
        tools,
        spool_dir: None,
    };
    compose_root_workspace(request).await.unwrap()
}

/// Builds the root toolset for a job with a working checkout: the child
/// workspace tools over W plus the coordinator's delegation tools, behind the
/// exact-write decider.
pub(super) async fn working_checkout_root(fixture: &fixture::Fixture) -> RootTools {
    working_checkout_root_with(fixture, None, None).await
}

/// The same root, carrying a web lane when the caller has one — the shape
/// `RootTools::build` composes for a job whose mode reaches the network — and
/// an MCP lane when it has one, the shape it composes for a job whose
/// configuration names servers.
pub(super) async fn working_checkout_root_with(
    fixture: &fixture::Fixture,
    web: Option<Arc<crate::agent::tools::web::WebTools>>,
    mcp: Option<Arc<crate::mcp::client::McpTools>>,
) -> RootTools {
    working_checkout_root_with_request(
        fixture,
        web,
        mcp,
        String::new(),
        saya_config::ResolvedTools::default(),
    )
    .await
}

/// The same root with the request text and the job's `[tools] allow` set the
/// evidence rule reads: the shape `RootTools::build` composes for a real job.
pub(super) async fn working_checkout_root_with_request(
    fixture: &fixture::Fixture,
    web: Option<Arc<crate::agent::tools::web::WebTools>>,
    mcp: Option<Arc<crate::mcp::client::McpTools>>,
    request: String,
    tools: saya_config::ResolvedTools,
) -> RootTools {
    let workspace = compose_root_data(fixture, tools.clone(), None).await;
    let coordinator = CodingTools {
        job: Arc::clone(&fixture.delegate_job),
        databases: None,
        denials: crate::agent::tools::coding::owner_denial::DenialTally::default(),
    };
    let decider = workspace.approval;
    RootTools::WorkingCheckout(Box::new(DirectCodingTools::new(
        coordinator,
        workspace.tools,
        workspace.definitions,
        decider,
        None,
        &saya_config::HelperAgents::default(),
        web,
        mcp,
        None,
        request,
        tools,
    )))
}

fn find<'a>(definitions: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
    definitions
        .iter()
        .find(|definition| definition.name == name)
        .unwrap_or_else(|| panic!("{name} is advertised"))
}

fn has(defs: &[ToolDefinition], name: &str) -> bool {
    defs.iter().any(|d| d.name == name)
}

/// A working checkout whose composition has no root shell — config off, or the
/// mode `never` — offers none, so the data-file route cannot name it.
#[tokio::test]
async fn a_root_without_a_shell_does_not_offer_one() {
    let fixture = fixture::Fixture::new("direct-no-shell", &["tracked.txt"]).await;
    let root = working_checkout_root(&fixture).await;
    assert!(!root.offers_shell());
}

// authored, not executed
#[tokio::test]
async fn direct_root_advertises_child_workspace_tools_plus_optional_delegation() {
    let fixture = fixture::Fixture::new("direct-advertise", &["tracked.txt"]).await;
    let root = working_checkout_root(&fixture).await;
    let advertised = root.definitions();
    // The child workspace tools a child gets for the same paths.
    assert!(has(&advertised, "read"));
    assert!(has(&advertised, "write"));
    assert!(has(&advertised, "edit"));
    // This composition passes no root shell, so `shell` is not advertised.
    assert!(!has(&advertised, "shell"));
    assert!(has(&advertised, "glob"));
    assert!(has(&advertised, "grep"));
    assert!(has(&advertised, "checklist"));
    // The lean default withholds `list` (the shell covers it) and `helpers`
    // (only a request that asks for one advertises it).
    assert!(!has(&advertised, "list"));
    assert!(!has(&advertised, "helpers"));
    // The child-facing check tools stay available to children, never advertised
    // by the root. `checklist` is required; `helpers` is optional.
    for name in ["coding_check", "coding_skills"] {
        assert!(!has(&advertised, name), "{name} must stay hidden");
    }
}

#[tokio::test]
async fn a_stored_task_list_is_reinjected_as_the_next_turn_note() {
    let fixture = fixture::Fixture::new("direct-tasks-note", &["tracked.txt"]).await;
    let root = working_checkout_root(&fixture).await;
    assert!(root.turn_context_note().is_none());
    root.execute(
        "checklist",
        serde_json::json!({
            "tasks": [{"title": "parse records", "status": "in_progress"}]
        }),
    )
    .await
    .expect("checklist stores");
    let note = root
        .turn_context_note()
        .expect("the list rides the next turn");
    assert!(note.contains("source: session-tasks"), "{note}");
    assert!(note.contains("parse records"), "{note}");
    assert!(
        root.turn_context_note().is_none(),
        "a turn whose list did not change carries no note: the model already has the state"
    );
}

// authored, not executed
#[tokio::test]
async fn a_direct_root_workspace_write_lands_in_the_root_checkout() {
    let fixture = fixture::Fixture::new("direct-write", &["tracked.txt"]).await;
    let root = working_checkout_root(&fixture).await;
    let write = find(&root.definitions(), "write").clone();
    let inside = serde_json::json!({"path": "root_wrote.txt", "content": "root wrote"});
    // Admitted: a new file in the root's working checkout.
    assert!(root.approve(&write, &inside).await);
    let outcome = root.execute("write", inside).await;
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
async fn a_job_with_no_write_path_denies_a_write_with_the_no_write_path_class() {
    // The working-checkout fixture supplies the `write` definition the
    // decider is asked about; the read-only job is a separate one with no
    // approved write path.
    let working = fixture::Fixture::new("direct-no-write-path-w", &["tracked.txt"]).await;
    let definition = find(
        &working_checkout_root(&working).await.definitions(),
        "write",
    )
    .clone();
    let fixture = fixture::Fixture::new("direct-no-write-path", &[]).await;
    let root = RootTools::build(&fixture.delegate_job).await.unwrap().0;
    let call = serde_json::json!({"path": "tracked.txt", "content": "x"});
    assert!(!root.approve(&definition, &call).await);
    let detail = root
        .refusal_detail(&definition, &call)
        .expect("a read-only job words its write denial");
    assert_eq!(
        detail,
        "owner write denied: this toolset holds no working checkout; delegate the change to a \
         helper, which writes the working checkout."
    );
    // The repeat guard is per class and per run, so the checkout-less toolset
    // carries it too: a second denial of the same class collapses to the next
    // step.
    assert!(!root.approve(&definition, &call).await);
    let repeat = root.refusal_detail(&definition, &call).unwrap();
    assert_eq!(
        repeat,
        "repeated denials (no_write_path_approved ×2): delegate the change to a helper, which \
         writes the working checkout."
    );
}

// authored, not executed
#[tokio::test]
async fn a_coordination_tool_name_routes_to_the_coordinator_executor() {
    let fixture = fixture::Fixture::new("direct-route", &["tracked.txt"]).await;
    let root = working_checkout_root(&fixture).await;
    // Child-facing check tools refuse as unknown on the root, same as the loop.
    for name in ["coding_check", "coding_skills"] {
        assert!(
            matches!(
                root.execute(name, serde_json::json!({})).await,
                Err(ToolError::UnsupportedTool)
            ),
            "{name} must refuse as unknown"
        );
    }
}

// The root's `shell` is exercised in `root_shell_tests`; a root composed
// without one advertises none at all (asserted above).

/// No supervisor tool is on either root variant: neither advertises one, and
/// executing one on the root refuses as unknown.
#[tokio::test]
async fn neither_root_variant_advertises_or_executes_the_retired_supervisor_tools() {
    let working_fixture =
        fixture::Fixture::new("direct-retired-supervisors-w", &["tracked.txt"]).await;
    let working = working_checkout_root(&working_fixture).await;
    let read_only_fixture = fixture::Fixture::new("direct-retired-supervisors", &[]).await;
    let no_write = RootTools::build(&read_only_fixture.delegate_job)
        .await
        .unwrap()
        .0;
    let arguments = serde_json::json!({"agent_id": "x", "tasks": [], "message": "m"});
    for name in [
        "delegate",
        "await_children",
        "message_child",
        "cancel_child",
    ] {
        assert!(
            !has(&working.definitions(), name),
            "{name} must not be advertised on a working checkout"
        );
        assert!(
            !has(&no_write.definitions(), name),
            "{name} must not be advertised without a write path"
        );
        assert!(
            matches!(
                working.execute(name, arguments.clone()).await,
                Err(ToolError::UnsupportedTool)
            ),
            "{name} must refuse as unknown on a working checkout"
        );
        assert!(
            matches!(
                no_write.execute(name, arguments.clone()).await,
                Err(ToolError::UnsupportedTool)
            ),
            "{name} must refuse as unknown without a write path"
        );
    }
}

/// A helper composes the job's narrowed set minus `helpers`: only the root may
/// delegate, so a name the list holds that a helper must not reach — the
/// delegation tool itself — is removed from the child's composition even when
/// the list names it.
#[tokio::test]
async fn a_stated_list_narrows_the_helper_composition() {
    let fixture = fixture::Fixture::new("direct-tools-narrow", &["tracked.txt"]).await;
    let stated = ["helpers".to_string(), "read".to_string()];
    let workspace = compose_root_with(
        &fixture,
        saya_config::ResolvedTools::from_stated(Some(&stated)).unwrap(),
    )
    .await;
    let names: Vec<&str> = workspace
        .definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["read"],
        "a helper advertises the job's set minus `helpers`, and nothing it did not name"
    );
}
