//! Tests for the read-only agent contract tools (slice 2b-3a).
//!
//! These exercise the `contract_search` and `contract_read` tools end-to-end
//! through the `ToolExecutor` surface the agent uses, against a real
//! `SqliteStateStore`. They assert the privacy gate, the store-failure
//! degradation, the absence of the opaque identity, and the `ToolDefinition`
//! invariants — see .claude/specs/spec-2b3a-agent-contract-tools.md §4.

use super::*;
use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutor};
use saya_connectors::DatabaseConnector;
use saya_store::{ContractStore, ProposeClaim, ProposeOutcome, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, ConnectionError, DatabaseObjectKind,
    DatabaseObjectRef, DatabaseProfile, ProfileIdentity, QueryRequest, QueryResult, SchemaTree,
    SqlDialect,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// A no-op connector: the contract tools never touch a live database, so the
/// registry only needs an entry to resolve a profile name + identity from.
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
        "saya-contract-tools-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn profile_identity(name: &str) -> ProfileIdentity {
    crate::profile_identity::profile_identity(
        name,
        &DatabaseProfile::DuckDb {
            path: "contract-tools.duckdb".into(),
            read_only: Some(true),
        },
        Path::new("/contract-tools-test/connections.toml"),
    )
}

/// A registry whose primary entry carries a real profile identity, so the
/// contract tools can resolve the connection name to the store's identity.
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

async fn remember_confirmed(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    payload: ClaimPayload,
) -> ClaimId {
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: crate::commands::unobserved_fingerprint(),
        payload,
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
    };
    match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    }
}

async fn remember_candidate(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    payload: ClaimPayload,
) -> ClaimId {
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: crate::commands::unobserved_fingerprint(),
        payload,
        origin: ClaimOrigin::AssistantInferred,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
    };
    match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    }
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

// ---------------------------------------------------------------------------
// Test 1: contract_search returns a confirmed claim; a candidate is absent.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn contract_search_returns_confirmed_and_hides_candidates() {
    let root = temp_root("search_confirmed");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let obj = object_ref(&identity, "orders");
    let _confirmed =
        remember_confirmed(&store, &obj, ClaimPayload::table_alias("orders").unwrap()).await;
    let _candidate = remember_candidate(
        &store,
        &obj,
        ClaimPayload::table_alias("secret_candidate").unwrap(),
    )
    .await;

    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store),
    );
    let res = tools
        .execute("contract_search", serde_json::json!({"terms": ["orders"]}))
        .await
        .expect("contract_search should succeed");

    let contracts = res
        .get("contracts")
        .and_then(|v| v.as_array())
        .expect("result carries a `contracts` array");
    assert_eq!(contracts.len(), 1, "one matched object");
    let claims = contracts[0]["claims"].as_array().expect("claims array");
    assert!(
        claims
            .iter()
            .any(|c| c["kind"] == "table_alias" && c["value"] == "orders"),
        "confirmed alias should appear: {res}"
    );
    assert!(
        !claims.iter().any(|c| c["value"] == "secret_candidate"),
        "candidate alias must not appear: {res}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 2: contract_read returns one object's contract; malformed table errors.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn contract_read_returns_one_contract_and_rejects_malformed_table() {
    let root = temp_root("read_one");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let obj = object_ref(&identity, "orders");
    let _id = remember_confirmed(
        &store,
        &obj,
        ClaimPayload::table_description("sales fact table").unwrap(),
    )
    .await;

    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store.clone()),
    );

    let res = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.orders"}),
        )
        .await
        .expect("contract_read should succeed");
    let contract = res.get("contract").expect("one contract");
    assert_eq!(contract["object"], "catalog.public.orders");
    assert!(
        contract["claims"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "table_description" && c["value"] == "sales fact table"),
        "stored description should appear: {res}"
    );

    // A malformed table is a typed tool error, not a panic or an untyped string.
    // `ToolError` (in saya-agent, untouchable from this slice) has no variant that
    // names the expected form, so the form is named in the tool description the
    // model reads; the typed error is `InvalidQueryArguments` (SPEC REVIEW 2b-3a).
    let err = tools
        .execute("contract_read", serde_json::json!({"table": "orders"}))
        .await
        .expect_err("malformed table should be a typed error");
    assert!(
        matches!(err, ToolError::InvalidQueryArguments),
        "malformed table should be InvalidQueryArguments: {err}"
    );

    let defs = DatabaseTools::definitions(true, true);
    let read_def = defs
        .iter()
        .find(|t| t.name == "contract_read")
        .expect("contract_read is defined");
    assert!(
        read_def.description.contains("catalog.schema"),
        "the description names the expected form: {}",
        read_def.description
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 3: unknown property, non-string connection, 17-entry terms.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn contract_tool_argument_validation_errors_are_typed() {
    let identity = profile_identity("primary");
    let tools =
        DatabaseTools::with_registry(registry_with_primary("primary", &identity), 100, true, None);

    let err = tools
        .execute(
            "contract_search",
            serde_json::json!({"terms": ["x"], "bogus": 1}),
        )
        .await
        .expect_err("unknown property must be rejected");
    assert!(matches!(err, ToolError::UnsupportedProperty), "got: {err}");

    let err = tools
        .execute(
            "contract_search",
            serde_json::json!({"terms": ["x"], "connection": 7}),
        )
        .await
        .expect_err("non-string connection must be rejected");
    assert!(matches!(err, ToolError::ConnectionNotString), "got: {err}");

    let terms: Vec<String> = (0..17).map(|i| format!("t{i}")).collect();
    let err = tools
        .execute("contract_search", serde_json::json!({"terms": terms}))
        .await
        .expect_err("17 terms must be rejected");
    assert!(
        matches!(err, ToolError::InvalidQueryArguments),
        "too many terms should be InvalidQueryArguments: {err}"
    );

    // terms not an array of strings is also a typed error.
    let err = tools
        .execute("contract_search", serde_json::json!({"terms": "orders"}))
        .await
        .expect_err("terms-as-string must be rejected");
    assert!(
        matches!(err, ToolError::InvalidQueryArguments),
        "got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: no opaque identity in any tool result.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn no_opaque_profile_identity_in_any_tool_result() {
    let root = temp_root("no_identity");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let obj = object_ref(&identity, "orders");
    let _id = remember_confirmed(&store, &obj, ClaimPayload::table_alias("orders").unwrap()).await;

    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store),
    );

    let search = tools
        .execute("contract_search", serde_json::json!({"terms": ["orders"]}))
        .await
        .unwrap();
    let read = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.orders"}),
        )
        .await
        .unwrap();

    let identity_str = identity.as_str();
    assert_eq!(identity_str.len(), 66);
    assert!(identity_str.starts_with("p-"));
    let search_text = serde_json::to_string(&search).unwrap();
    let read_text = serde_json::to_string(&read).unwrap();
    assert!(
        !search_text.contains(identity_str),
        "identity leaked into contract_search: {search_text}"
    );
    assert!(
        !read_text.contains(identity_str),
        "identity leaked into contract_read: {read_text}"
    );
    // The profile NAME the model may see is present, not the identity.
    assert!(search_text.contains("\"profile\":\"primary\""));
    assert!(read_text.contains("\"profile\":\"primary\""));

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 5: privacy gate closed -> both tools return empty, no claims.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn privacy_gate_closed_returns_empty_without_claims() {
    let root = temp_root("privacy_closed");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let obj = object_ref(&identity, "orders");
    let _id = remember_confirmed(
        &store,
        &obj,
        ClaimPayload::table_alias("secret_alias").unwrap(),
    )
    .await;

    // allow_query_data = false models the closed privacy gate.
    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        false,
        Some(store),
    );

    let res = tools
        .execute(
            "contract_search",
            serde_json::json!({"terms": ["secret_alias"]}),
        )
        .await
        .expect("privacy gate returns empty, not an error");
    assert!(
        res.get("contracts")
            .map(|c| c.as_array().unwrap().is_empty())
            .unwrap_or(true),
        "no contracts when the gate is closed: {res}"
    );
    let text = serde_json::to_string(&res).unwrap();
    assert!(!text.contains("secret_alias"));

    let res = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.orders"}),
        )
        .await
        .expect("privacy gate returns empty, not an error");
    assert!(
        res.get("contract").is_none(),
        "no contract when closed: {res}"
    );
    let text = serde_json::to_string(&res).unwrap();
    assert!(!text.contains("secret_alias"));

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 6: unopenable store -> empty result, never Err.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unopenable_store_returns_empty_result_not_error() {
    let root = temp_root("unopenable");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);
    let identity = profile_identity("primary");

    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store),
    );

    let search = tools
        .execute("contract_search", serde_json::json!({"terms": ["orders"]}))
        .await
        .expect("store failure must be an empty result, not Err");
    assert!(
        search
            .get("contracts")
            .map(|c| c.as_array().unwrap().is_empty())
            .unwrap_or(true),
        "empty contracts on store failure: {search}"
    );

    let read = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.orders"}),
        )
        .await
        .expect("store failure must be an empty result, not Err");
    assert!(
        read.get("contract").is_none(),
        "no contract on store failure: {read}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 7: ToolDefinition invariants — read-only, no approval, not writable.
// ---------------------------------------------------------------------------
#[test]
fn contract_tool_definitions_are_read_only_unapproved_and_not_writable() {
    let tools = DatabaseTools::definitions(true, true);
    let search = tools
        .iter()
        .find(|t| t.name == "contract_search")
        .expect("contract_search is registered when a store is present and data is allowed");
    let read = tools
        .iter()
        .find(|t| t.name == "contract_read")
        .expect("contract_read is registered");

    for tool in [search, read] {
        assert!(tool.read_only, "{} must be read_only", tool.name);
        assert!(
            !tool.effect.requires_approval,
            "{} must not require approval",
            tool.name
        );
        assert!(
            !tool.effect.external_side_effect,
            "{} must have no external side effect",
            tool.name
        );
        assert!(
            tool.effect.database_data,
            "{} is database-derived and subject to the sharing gate",
            tool.name
        );
    }
}

// ---------------------------------------------------------------------------
// Test 7b: availability gate — tools are hidden without a store or when the
// privacy gate is closed (spec §3: hide, do not advertise-empty).
// ---------------------------------------------------------------------------
#[test]
fn contract_tools_are_hidden_without_a_store_or_when_the_gate_is_closed() {
    let with_store_and_gate = DatabaseTools::definitions(true, true);
    assert!(
        with_store_and_gate
            .iter()
            .any(|t| t.name == "contract_search")
    );
    assert!(
        with_store_and_gate
            .iter()
            .any(|t| t.name == "contract_read")
    );

    let no_store = DatabaseTools::definitions(true, false);
    assert!(!no_store.iter().any(|t| t.name == "contract_search"));
    assert!(!no_store.iter().any(|t| t.name == "contract_read"));

    let gate_closed = DatabaseTools::definitions(false, true);
    assert!(!gate_closed.iter().any(|t| t.name == "contract_search"));
    assert!(!gate_closed.iter().any(|t| t.name == "contract_read"));
}

// ---------------------------------------------------------------------------
// Test 8: contract_read flags truncation past the per-object claim bound.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn contract_read_truncates_claims_past_the_bound_and_says_so() {
    let root = temp_root("read_truncated");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let obj = object_ref(&identity, "wide");
    // More confirmed claims than the 12-per-object default bound.
    for i in 0..20 {
        let _ = remember_confirmed(
            &store,
            &obj,
            ClaimPayload::table_alias(format!("a{i}")).unwrap(),
        )
        .await;
    }

    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store),
    );
    let res = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.wide"}),
        )
        .await
        .expect("contract_read should succeed");
    let contract = res.get("contract").expect("one contract");
    let claims = contract["claims"].as_array().expect("claims array");
    assert!(
        claims.len() <= 12,
        "claims bounded to 12, got {}: {res}",
        claims.len()
    );
    assert!(
        contract["truncated"] == serde_json::Value::Bool(true),
        "truncation must be flagged explicitly: {res}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 9: contract_read on an object with no contract returns empty + reason.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn contract_read_on_unknown_object_returns_empty_with_reason() {
    let root = temp_root("read_unknown");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let tools = DatabaseTools::with_registry(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store),
    );

    let res = tools
        .execute(
            "contract_read",
            serde_json::json!({"table": "catalog.public.missing"}),
        )
        .await
        .expect("no contract is an empty result, not an error");
    assert!(res.get("contract").is_none(), "no contract object: {res}");
    let reason = res
        .get("reason")
        .and_then(|v| v.as_str())
        .expect("a short reason explains the empty result");
    assert!(!reason.is_empty());

    let _ = fs::remove_dir_all(root);
}
