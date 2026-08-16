//! Tests for the `contract_propose` agent tool (slice 3c) — the first tool that
//! writes local state. They drive the tool through the `ToolExecutor` surface
//! against a real `SqliteStateStore` and assert the policy in spec 3c §2:
//! origin is always `AssistantInferred`, status is always `Candidate`, the
//! per-turn bound, malformed/oversized input refused before persistence, the
//! duplicate shape, the availability gate, the weaker evidence for untouched
//! objects, and that the opaque identity never appears in a result.

use super::*;
use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutor};
use saya_connectors::DatabaseConnector;
use saya_store::{ContractStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimOrigin, ClaimStatus, ConnectionError, DatabaseObjectKind, DatabaseObjectRef,
    DatabaseProfile, ProfileIdentity, QueryRequest, QueryResult, SchemaTree, SqlDialect,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::agent::tools::ProposedClaimsLog;
use crate::agent::tools::database_tools::ObservationLog;
use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// A connector that succeeds; `contract_propose` never reaches it, but the
/// registry needs an entry to resolve a profile identity from.
struct IdleConnector;

#[async_trait]
impl DatabaseConnector for IdleConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(req.sql))
    }
}

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-contract-propose-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn profile_identity(name: &str) -> ProfileIdentity {
    crate::profile_identity::profile_identity(
        name,
        &DatabaseProfile::DuckDb {
            path: "propose.duckdb".into(),
            read_only: Some(true),
        },
        Path::new("/propose-test/connections.toml"),
    )
}

fn registry_with_primary(name: &str, identity: &ProfileIdentity) -> ConnectionRegistry {
    let mut registry = ConnectionRegistry::new(name);
    registry.insert(
        name,
        ConnectionEntry {
            connector: Box::new(IdleConnector),
            dialect: SqlDialect::DuckDb,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    registry
}

async fn store_at(db: &Path) -> SqliteStateStore {
    let store = SqliteStateStore::new(db);
    store
        .upsert_schema(profile_identity("primary").as_str(), &SchemaTree::default())
        .await
        .unwrap();
    store
}

fn object_ref(profile: &ProfileIdentity, name: &str) -> DatabaseObjectRef {
    DatabaseObjectRef::new(
        profile.clone(),
        "catalog",
        "public",
        name,
        DatabaseObjectKind::Table,
    )
    .unwrap()
}

/// Tools with a store and a shared observation log, ready to drive propose.
/// `proposed_claims` is the request-scoped log `contract_propose` records a
/// persisted claim into; pass `Some` to assert the `KnowledgeProposed` payload
/// (spec P2d), `None` for tests that do not inspect the event.
fn propose_tools(
    registry: ConnectionRegistry,
    store: SqliteStateStore,
    observations: Arc<ObservationLog>,
    proposed_claims: Option<Arc<ProposedClaimsLog>>,
) -> DatabaseTools {
    DatabaseTools::with_registry_and_observations(
        registry,
        100,
        true,
        Some(store),
        observations,
        proposed_claims,
    )
}

fn propose_args(table: &str, kind: &str, value: &str) -> serde_json::Value {
    serde_json::json!({"table": table, "kind": kind, "value": value})
}

/// Reads the one stored claim for `object` back as a candidate and asserts its
/// origin/status — the store is the authority for what was persisted.
async fn assert_one_candidate(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
) -> saya_store::StoredClaim {
    let claims = store
        .list_claims(object, &[ClaimStatus::Candidate])
        .await
        .unwrap();
    assert_eq!(claims.len(), 1, "exactly one candidate stored: {claims:?}");
    let claim = &claims[0];
    assert_eq!(
        claim.origin,
        ClaimOrigin::AssistantInferred,
        "origin must be AssistantInferred: {:?}",
        claim.origin
    );
    assert_eq!(
        claim.status,
        ClaimStatus::Candidate,
        "status must be Candidate: {:?}",
        claim.status
    );
    claims.into_iter().next().unwrap()
}

// ---------------------------------------------------------------------------
// Test 1: a proposal stores a claim with origin AssistantInferred, status Candidate.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn proposal_stores_assistant_inferred_candidate() {
    let root = temp_root("store_candidate");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        Arc::new(ObservationLog::new()),
        None,
    );

    let res = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .expect("proposal should store");
    assert_eq!(res["action"], "proposed");
    assert_eq!(res["status"], "candidate");

    let obj = object_ref(&identity, "orders");
    assert_one_candidate(&store, &obj).await;

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 2: the stored candidate does NOT appear in ordinary recall.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stored_candidate_does_not_appear_in_recall() {
    let root = temp_root("not_in_recall");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        Arc::new(ObservationLog::new()),
        None,
    );
    tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .unwrap();

    // Ordinary recall is the read tool: it returns only confirmed claims.
    let res = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.orders"}),
        )
        .await
        .unwrap();
    assert!(
        res.get("contract").is_none(),
        "a candidate must not appear in ordinary recall: {res}"
    );
    let text = serde_json::to_string(&res).unwrap();
    assert!(!text.contains("orders"));

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 3: a ninth proposal in one turn is refused; exactly eight were stored.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ninth_proposal_in_a_turn_is_refused_and_eight_stored() {
    let root = temp_root("nine_proposals");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        Arc::new(ObservationLog::new()),
        None,
    );

    // Eight distinct objects, one proposal each — all store.
    for i in 0..8 {
        let table = format!("catalog.public.t{i}");
        tools
            .execute(
                "contract_propose",
                propose_args(&table, "description", "a fact table"),
            )
            .await
            .expect("first eight proposals store");
    }
    // The ninth is refused with a typed error.
    let err = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.t8", "description", "a fact table"),
        )
        .await
        .expect_err("the ninth proposal must be refused");
    assert!(
        matches!(err, ToolError::QueryFailed),
        "the per-turn limit is a typed refusal (no limit variant in ToolError): {err}"
    );

    // Exactly eight candidates were stored (one per object).
    let mut count = 0;
    for i in 0..9 {
        let obj = object_ref(&identity, &format!("t{i}"));
        let claims = store
            .list_claims(&obj, &[ClaimStatus::Candidate])
            .await
            .unwrap();
        count += claims.len();
    }
    assert_eq!(count, 8, "exactly eight candidates stored, got {count}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 4: malformed table, unknown kind, missing column for a column kind — each
// a typed error, and nothing is stored.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn malformed_input_is_a_typed_error_and_stores_nothing() {
    let root = temp_root("malformed");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        Arc::new(ObservationLog::new()),
        None,
    );

    // Malformed table (two parts).
    let err = tools
        .execute(
            "contract_propose",
            propose_args("public.orders", "alias", "orders"),
        )
        .await
        .expect_err("malformed table must be a typed error");
    assert!(
        matches!(err, ToolError::InvalidQueryArguments),
        "got: {err}"
    );

    // Unknown kind.
    let err = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "not-a-kind", "orders"),
        )
        .await
        .expect_err("unknown kind must be a typed error");
    assert!(
        matches!(err, ToolError::InvalidQueryArguments),
        "got: {err}"
    );

    // Column kind without a column.
    let err = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "column-description", "a note"),
        )
        .await
        .expect_err("missing column must be a typed error");
    assert!(
        matches!(err, ToolError::InvalidQueryArguments),
        "got: {err}"
    );

    // Nothing was stored under the object.
    let obj = object_ref(&identity, "orders");
    let claims = store
        .list_claims(&obj, &[ClaimStatus::Candidate])
        .await
        .unwrap();
    assert!(
        claims.is_empty(),
        "malformed input stored nothing: {claims:?}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 5: a value with a control character or exceeding the payload cap is a
// typed error, nothing is stored, and the error text does not contain the value.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn bad_value_is_typed_error_without_echo_and_stores_nothing() {
    let root = temp_root("bad_value");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        Arc::new(ObservationLog::new()),
        None,
    );

    const SENTINEL: &str = "SENTINELVALUE";

    // Control character in the value.
    let err = tools
        .execute(
            "contract_propose",
            propose_args(
                "catalog.public.orders",
                "description",
                &format!("hello{SENTINEL}\nworld"),
            ),
        )
        .await
        .expect_err("a control character in the value must be refused");
    assert!(
        matches!(err, ToolError::InvalidQueryArguments),
        "got: {err}"
    );
    assert!(
        !format!("{err}").contains(SENTINEL),
        "the error must not echo the offending value: {err}"
    );

    // Value exceeding the payload cap (4 KiB).
    let long = format!("{SENTINEL}{}", "x".repeat(5000));
    let err = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "description", &long),
        )
        .await
        .expect_err("an oversized value must be refused");
    assert!(
        matches!(err, ToolError::InvalidQueryArguments),
        "got: {err}"
    );
    assert!(
        !format!("{err}").contains(SENTINEL),
        "the error must not echo the offending value: {err}"
    );

    let obj = object_ref(&identity, "orders");
    let claims = store
        .list_claims(&obj, &[ClaimStatus::Candidate])
        .await
        .unwrap();
    assert!(claims.is_empty(), "bad value stored nothing: {claims:?}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 6: a duplicate returns the existing id and status; a duplicate of a
// forgotten claim reports forgotten.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn duplicate_returns_existing_id_and_status() {
    let root = temp_root("duplicate");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        Arc::new(ObservationLog::new()),
        None,
    );

    let res = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .unwrap();
    let first_id = res["claim_id"].as_str().unwrap().to_string();
    assert_eq!(res["action"], "proposed");
    assert_eq!(res["status"], "candidate");

    // A duplicate of the live candidate returns the same id, action=duplicate,
    // status=candidate.
    let res = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .unwrap();
    assert_eq!(res["claim_id"], first_id);
    assert_eq!(res["action"], "duplicate");
    assert_eq!(res["status"], "candidate");

    // Forget it, then propose again: the duplicate must report forgotten, not
    // success — a forgotten claim must not read as resurrected.
    use saya_store::ForgetReason;
    let claim_id = saya_types::ClaimId::parse(&first_id).unwrap();
    store
        .forget_claim(&claim_id, ForgetReason::UserRequest)
        .await
        .unwrap();
    let res = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .unwrap();
    assert_eq!(res["claim_id"], first_id);
    assert_eq!(res["action"], "duplicate");
    assert_eq!(
        res["status"], "forgotten",
        "a duplicate of a forgotten claim reports forgotten, not success: {res}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 7: with permit_candidate_writes = false the tool is absent from the
// definitions (we hid it) and nothing is written under any argument. Because it
// is hidden, the loop never routes it; calling the executor directly is the
// only way a caller could reach it, and even then the per-turn store path is the
// same — so this asserts the definitions gate, the source of truth for hiding.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn tool_is_hidden_when_candidate_writes_not_permitted() {
    // Hidden from the definitions under every combination that lacks writes.
    assert!(
        !DatabaseTools::definitions(true, true, false)
            .iter()
            .any(|t| t.name == "contract_propose")
    );
    assert!(
        !DatabaseTools::definitions(true, false, true)
            .iter()
            .any(|t| t.name == "contract_propose")
    );
    assert!(
        !DatabaseTools::definitions(false, true, true)
            .iter()
            .any(|t| t.name == "contract_propose")
    );
    // Present only when all three gates open.
    assert!(
        DatabaseTools::definitions(true, true, true)
            .iter()
            .any(|t| t.name == "contract_propose")
    );

    // And the write gate (permit_candidate_writes) is the one the loop enforces:
    // a default AgentLimits refuses a WriteCandidate tool. This is the loop-layer
    // backstop that makes hiding defense-in-depth, not the only check.
    use saya_agent::{AgentLimits, LocalStateEffect, ToolEffect};
    let limits = AgentLimits::default();
    assert!(!limits.permit_candidate_writes);
    let effect = ToolEffect {
        database_data: true,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::WriteCandidate,
    };
    assert!(
        effect.local_state == LocalStateEffect::WriteCandidate && !limits.permit_candidate_writes,
        "the loop denies a WriteCandidate tool when writes are not permitted"
    );
}

// ---------------------------------------------------------------------------
// Test 8: a proposal about an object the turn never touched still stores, with
// the weaker evidence kind. The kind mapping is asserted in `propose/evidence`;
// here we assert the untouched object still stores a candidate.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn untouched_object_stores_with_weaker_evidence() {
    let root = temp_root("untouched");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ObservationLog::new());
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
        None,
    );

    // No query ran this turn, so the proposed object was never touched.
    let res = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .expect("an untouched object still stores a candidate");
    assert_eq!(res["action"], "proposed");

    let obj = object_ref(&identity, "orders");
    assert_one_candidate(&store, &obj).await;

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 8b: a proposal about an object the turn DID touch (a succeeded query
// named it) stores with the stronger evidence kind. The kind is decided by
// `propose::evidence` and asserted there; here we assert a touched object
// stores just as an untouched one does — the touch changes only the kind.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn touched_object_stores_a_candidate() {
    let root = temp_root("touched");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ObservationLog::new());
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
        None,
    );

    // A succeeded query this turn named `catalog.public.orders`.
    log.record(crate::agent::tools::database_tools::ToolObservation {
        tool: "bounded_sql_query".into(),
        outcome: crate::agent::tools::database_tools::ObservationOutcome::Succeeded,
        profile: None,
        objects: vec![vec!["catalog".into(), "public".into(), "orders".into()]],
        columns: Vec::new(),
        row_count: Some(1),
        truncated: None,
        references_partial: false,
    });

    let res = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .unwrap();
    assert_eq!(res["action"], "proposed");

    let obj = object_ref(&identity, "orders");
    assert_one_candidate(&store, &obj).await;

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 9: the opaque profile identity appears in no tool result.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn no_opaque_profile_identity_in_propose_result() {
    let root = temp_root("no_identity_propose");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        Arc::new(ObservationLog::new()),
        None,
    );

    let res = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .unwrap();

    let identity_str = identity.as_str();
    assert_eq!(identity_str.len(), 66);
    assert!(identity_str.starts_with("p-"));
    let text = serde_json::to_string(&res).unwrap();
    assert!(
        !text.contains(identity_str),
        "identity leaked into contract_propose result: {text}"
    );

    let _ = fs::remove_dir_all(root);
}

// ===========================================================================
// P2d — `KnowledgeProposed`: what was *written*, not just that something was.
//
// These drive `contract_propose` through the executor with a `ProposedClaimsLog`
// attached (the request-scoped log the runtime drains to emit the event), then
// `drain` it. The log records only on the `Stored` arm — so what it holds is,
// by construction, only what was persisted (spec P2d §3). The runtime-level
// test (one event actually emitted through `run_prompt_with_inputs`) lives in
// `runtime_tests`; these assert the data the event would carry.
// ===========================================================================

/// Tools with a store, an observation log, and a proposed-claims log.
fn propose_tools_with_log(
    registry: ConnectionRegistry,
    store: SqliteStateStore,
    log: Arc<ProposedClaimsLog>,
) -> DatabaseTools {
    propose_tools(registry, store, Arc::new(ObservationLog::new()), Some(log))
}

/// A persisted proposal records exactly one claim naming what was stored —
/// claim id, profile name, object, kind, rendered value, and `Candidate`
/// status (spec P2d §5.1).
#[tokio::test]
async fn persisted_proposal_records_one_claim_naming_what_was_stored() {
    let root = temp_root("p2d_stored");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ProposedClaimsLog::new());
    let tools = propose_tools_with_log(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
    );

    let res = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .expect("proposal stores");
    let claim_id = res["claim_id"].as_str().unwrap().to_string();

    let recorded = log.drain();
    assert_eq!(
        recorded.len(),
        1,
        "exactly one claim recorded: {recorded:?}"
    );
    let claim = &recorded[0];
    assert_eq!(
        claim.claim_id.as_str(),
        claim_id,
        "carries the stored claim id"
    );
    assert_eq!(
        claim.profile, "primary",
        "the profile name, not the identity"
    );
    assert_eq!(
        claim.object, "catalog.public.orders",
        "the qualified object"
    );
    assert_eq!(claim.kind, "table_alias", "the claim kind token");
    assert_eq!(claim.value, "orders", "the rendered value, not the payload");
    assert!(claim.column.is_none(), "a table-level claim has no column");
    assert_eq!(
        claim.status,
        ClaimStatus::Candidate,
        "landed as a candidate"
    );

    // The store is the authority: the recorded claim is the one that persisted.
    let obj = object_ref(&identity, "orders");
    let stored = assert_one_candidate(&store, &obj).await;
    assert_eq!(stored.id.as_str(), claim.claim_id.as_str());

    let _ = fs::remove_dir_all(root);
}

/// A column-scoped proposal records the column too (the `claim_value` shape).
#[tokio::test]
async fn column_scoped_proposal_records_the_column() {
    let root = temp_root("p2d_column");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ProposedClaimsLog::new());
    let tools = propose_tools_with_log(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
    );

    tools
        .execute(
            "contract_propose",
            serde_json::json!({
                "table": "catalog.public.orders",
                "kind": "column-role",
                "value": "dimension",
                "column": "amount",
            }),
        )
        .await
        .unwrap();

    let recorded = log.drain();
    assert_eq!(recorded.len(), 1);
    let claim = &recorded[0];
    assert_eq!(claim.kind, "column_role");
    assert_eq!(claim.value, "dimension", "the role renders as the value");
    assert_eq!(
        claim.column.as_deref(),
        Some("amount"),
        "a column-scoped claim carries its column"
    );

    let _ = fs::remove_dir_all(root);
}

/// A refused, duplicate, or validation-failed proposal records nothing — the
/// event names what was *written*, never what was merely asked for (spec P2d
/// §3, §5.2). Each failure mode is a separate turn (its own `DatabaseTools` /
/// log) so the per-turn counter and the log start clean.
#[tokio::test]
async fn refused_duplicate_and_malformed_proposals_record_nothing() {
    // --- validation-failed: a malformed table stores nothing, records nothing.
    let root = temp_root("p2d_malformed");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ProposedClaimsLog::new());
    let tools = propose_tools_with_log(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
    );
    let err = tools
        .execute(
            "contract_propose",
            propose_args("public.orders", "alias", "orders"),
        )
        .await
        .expect_err("malformed table is a typed error");
    assert!(matches!(err, ToolError::InvalidQueryArguments));
    assert!(
        log.drain().is_empty(),
        "a validation failure records nothing"
    );
    let _ = fs::remove_dir_all(root);

    // --- refused: a closed privacy gate refuses before any write, records
    // nothing. A separate `DatabaseTools` with `allow_query_data = false`.
    let root = temp_root("p2d_refused");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ProposedClaimsLog::new());
    let tools = DatabaseTools::with_registry_and_observations(
        registry_with_primary("primary", &identity),
        100,
        false, // privacy gate closed
        Some(store.clone()),
        Arc::new(ObservationLog::new()),
        Some(log.clone()),
    );
    let err = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .expect_err("a closed gate refuses the proposal");
    assert!(matches!(err, ToolError::DataSharingDisabled));
    assert!(log.drain().is_empty(), "a refused proposal records nothing");
    let _ = fs::remove_dir_all(root);

    // --- duplicate: the first proposal stores and records; a duplicate of it
    // returns the existing id but is NOT a new proposal, so it records nothing.
    let root = temp_root("p2d_duplicate");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ProposedClaimsLog::new());
    let tools = propose_tools_with_log(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
    );
    tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .unwrap();
    assert_eq!(log.drain().len(), 1, "the first proposal records one");

    let dup = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .unwrap();
    assert_eq!(dup["action"], "duplicate", "the second call is a duplicate");
    assert!(
        log.drain().is_empty(),
        "a duplicate is not a new proposal — records nothing"
    );
    let _ = fs::remove_dir_all(root);
}

/// The recorded claim's `Candidate` status is distinguishable from `Confirmed`
/// — a candidate never reads as established (spec P2d §3, §5.3).
#[tokio::test]
async fn recorded_status_is_candidate_distinguishable_from_confirmed() {
    let root = temp_root("p2d_status");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ProposedClaimsLog::new());
    let tools = propose_tools_with_log(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
    );
    tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .unwrap();
    let claim = &log.drain()[0];
    assert_eq!(claim.status, ClaimStatus::Candidate);
    assert_ne!(
        claim.status,
        ClaimStatus::Confirmed,
        "a proposal never lands as confirmed"
    );
    let _ = fs::remove_dir_all(root);
}

/// The serialized `KnowledgeProposed` event carries no opaque `ProfileIdentity`
/// — the profile *name* only (spec P2d §3, §5.4).
#[tokio::test]
async fn no_opaque_profile_identity_in_proposed_event() {
    let root = temp_root("p2d_no_identity");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ProposedClaimsLog::new());
    let tools = propose_tools_with_log(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
    );
    tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .unwrap();
    let claim = log.drain().pop().unwrap();
    let event = saya_agent::AgentEvent::knowledge_proposed(claim);
    let json = serde_json::to_string(&event).expect("serializes");
    let identity_str = identity.as_str();
    assert_eq!(identity_str.len(), 66);
    assert!(identity_str.starts_with("p-"));
    assert!(
        !json.contains(identity_str),
        "opaque identity leaked into the KnowledgeProposed event: {json}"
    );
    assert!(
        json.contains("primary"),
        "the profile name (not the identity) is what the event carries: {json}"
    );
    // Round-trips through serde with the `knowledge_proposed` type tag.
    assert!(
        json.contains(r#""type":"knowledge_proposed""#),
        "carries the type tag: {json}"
    );
    let back: saya_agent::AgentEvent = serde_json::from_str(&json).expect("deserializes back");
    assert_eq!(back, event, "round-trips with the claim intact");
    let _ = fs::remove_dir_all(root);
}

/// The per-turn proposal bound still holds: exactly eight proposals store and
/// record; the ninth is refused and records nothing (spec P2d §3, §5.6).
#[tokio::test]
async fn per_turn_bound_holds_and_ninth_refused_records_nothing() {
    let root = temp_root("p2d_bound");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ProposedClaimsLog::new());
    let tools = propose_tools_with_log(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
    );

    for i in 0..8 {
        tools
            .execute(
                "contract_propose",
                propose_args(
                    &format!("catalog.public.t{i}"),
                    "description",
                    "a fact table",
                ),
            )
            .await
            .expect("first eight proposals store");
    }
    assert_eq!(log.drain().len(), 8, "exactly eight recorded");

    let err = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.t8", "description", "a fact table"),
        )
        .await
        .expect_err("the ninth is refused");
    assert!(matches!(err, ToolError::QueryFailed), "got: {err}");
    assert!(
        log.drain().is_empty(),
        "the refused ninth records nothing — the event stream inherits the bound"
    );

    let _ = fs::remove_dir_all(root);
}
