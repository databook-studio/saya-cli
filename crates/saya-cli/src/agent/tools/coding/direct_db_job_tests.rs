//! What a job surface decides about database calls: nothing moved. The lane is
//! the chat surface's alone, so `-p`, a background run and a helper keep the
//! decider they always composed — pinned here from the code the root calls.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ApprovalPolicy, ToolDefinition};
use saya_harness::coding::policy::{
    ChildAuthorityRequest, ChildRole, PlatformSupport, RootDelegationInput, RootDelegationScope,
};
use saya_types::{
    AgentId, Capabilities, InvocationId, JobId, ManagedCheckoutId, ProfileIdentity, RepositoryId,
    Sha256Digest, SnapshotId,
};
use serde_json::{Value, json};

use super::direct_db_lane_tests::{Recording, db_tool, query};
use super::direct_tests::working_checkout_root;
use crate::agent::tools::child::decider::ExactWriteChildApproval;
use crate::agent::tools::child::tests_root_compose_fixture as fixture;

/// Explicit host consent for the fixture's root scope: approves only
/// `coding_root_scope`.
struct RootConsent;

#[async_trait]
impl ApprovalDecider for RootConsent {
    async fn approve(&self, tool: &ToolDefinition, _: &Value) -> bool {
        tool.name == "coding_root_scope"
    }
}

/// A job surface keeps today's decisions: the read pair is refused where the
/// job selected no profile, the contract tools are refused, and neither is put
/// to the operator. Attaching the lane here would allow the contract tools and
/// ask about the query, which is the swap this unit does not make.
#[tokio::test]
async fn a_job_surface_keeps_todays_database_decisions() {
    let operator = Recording::new();
    let fixture = fixture::Fixture::asking(
        "direct-db-job",
        &["tracked.txt"],
        operator.clone(),
        ApprovalPolicy::Ask,
    )
    .await;
    let root = working_checkout_root(&fixture).await;

    assert!(
        root.db_lane().is_none(),
        "a job surface takes no database lane"
    );
    assert!(
        !root
            .approve(&db_tool("bounded_sql_query"), &query("demo"))
            .await,
        "no selected profile: the capability refuses the query"
    );
    assert!(
        !root
            .approve(&db_tool("contract_search"), &json!({ "terms": ["orders"] }))
            .await,
        "the contract tools stay refused on a job surface"
    );
    assert_eq!(
        operator.calls(),
        0,
        "no database call is put to the operator"
    );
}

/// The decider a job surface consults allows the read pair where its consent
/// selected a profile, and still refuses the contract tools.
#[tokio::test]
async fn a_job_surface_allows_the_read_pair_where_a_profile_was_selected() {
    let checkout = ManagedCheckoutId::parse("checkout").unwrap();
    let profile = ProfileIdentity::parse(&format!("p-{}", "0".repeat(64))).unwrap();
    let scope = RootDelegationScope::approve(
        RootDelegationInput {
            job_id: JobId::parse("job").unwrap(),
            agent_id: AgentId::parse("agent").unwrap(),
            invocation_id: InvocationId::parse("invocation").unwrap(),
            repository_id: RepositoryId::parse("repository").unwrap(),
            snapshot_id: SnapshotId::parse("snapshot").unwrap(),
            managed_checkout_id: Some(checkout.clone()),
            policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
            revocation_epoch: 7,
            capabilities: Capabilities::default(),
            database_profiles: vec![profile.clone()],
        },
        &RootConsent,
    )
    .await
    .unwrap();
    let view = scope
        .narrow(
            ChildAuthorityRequest::new(ChildRole::Analyst, Capabilities::default(), vec![profile])
                .unwrap(),
            PlatformSupport {
                workspace_edits: false,
                network_fetch: false,
                database_reads: true,
            },
        )
        .unwrap();
    let decider = ExactWriteChildApproval::new(&view, checkout);

    assert!(
        decider
            .approve(&db_tool("bounded_sql_query"), &query("demo"))
            .await,
        "a selected profile admits the bounded query"
    );
    assert!(
        !decider
            .approve(&db_tool("contract_search"), &json!({ "terms": ["orders"] }))
            .await,
        "the contract tools stay refused whatever the job selected"
    );
}
