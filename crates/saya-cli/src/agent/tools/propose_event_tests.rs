//! Regression and correctness tests for contract proposal evidence attachment:
//! a claim supplied this turn must not become its own evidence (spec slice 3c).

use async_trait::async_trait;
use saya_agent::ToolExecutor;
use saya_connectors::DatabaseConnector;
use saya_store::{EvidenceKind, SchemaStore, SqliteStateStore};
use saya_types::{
    ConnectionError, DatabaseProfile, ProfileIdentity, QueryRequest, QueryResult, SchemaTree,
    SqlDialect,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::database_tools::{DatabaseTools, ObservationLog, ObservationOutcome, ToolObservation};
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use crate::contracts::args::parse_qualified;

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
        "saya-contract-propose-event-{label}-{}-{stamp}",
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

fn propose_tools(
    registry: ConnectionRegistry,
    store: SqliteStateStore,
    observations: Arc<ObservationLog>,
) -> DatabaseTools {
    DatabaseTools::with_registry_and_observations(
        registry,
        100,
        true,
        Some(store),
        observations,
        None,
    )
}

fn propose_args(table: &str, kind: &str, value: &str) -> serde_json::Value {
    serde_json::json!({
        "table": table,
        "kind": kind,
        "value": value,
    })
}

// ---------------------------------------------------------------------------
// Test 1 (Regression): Claim supplied for object X this turn + query touched X
// + proposal about X -> UNTOUCHED (RepeatedObservation).
// The query was caused by the supplied claim, so it is not independent evidence.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn supplied_claim_object_touched_by_query_gets_untouched_evidence() {
    let root = temp_root("test1_supplied_touched");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ObservationLog::new());
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
    )
    .with_supplied_objects(vec!["catalog.public.orders".to_string()]);

    // A succeeded query this turn touched `catalog.public.orders`.
    log.record(ToolObservation {
        tool: "bounded_sql_query".into(),
        outcome: ObservationOutcome::Succeeded,
        profile: None,
        objects: vec![vec!["catalog".into(), "public".into(), "orders".into()]],
        columns: Vec::new(),
        row_count: Some(1),
        truncated: None,
        references_partial: false,
    });

    let q = parse_qualified("catalog.public.orders").unwrap();
    // A proposal about an object whose claims were supplied this turn must get UNTOUCHED
    // (RepeatedObservation), not TOUCHED (SuccessfulReadQuery), even though a query touched it.
    assert_eq!(
        tools.evidence_kind(&q),
        EvidenceKind::RepeatedObservation,
        "an object whose claims were supplied this turn must get UNTOUCHED even if touched"
    );

    // Also assert tool execution succeeds and stores the candidate.
    let res = tools
        .execute(
            "contract_propose",
            propose_args("catalog.public.orders", "alias", "orders"),
        )
        .await
        .unwrap();
    assert_eq!(res["action"], "proposed");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 2: No claim supplied for X + query touched X + proposal about X -> TOUCHED.
// Unchanged behaviour: without prior recall, a query touch earns the strong kind.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unsupplied_object_touched_by_query_gets_touched_evidence() {
    let root = temp_root("test2_unsupplied_touched");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ObservationLog::new());
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
    );

    // A succeeded query this turn touched `catalog.public.orders`.
    log.record(ToolObservation {
        tool: "bounded_sql_query".into(),
        outcome: ObservationOutcome::Succeeded,
        profile: None,
        objects: vec![vec!["catalog".into(), "public".into(), "orders".into()]],
        columns: Vec::new(),
        row_count: Some(1),
        truncated: None,
        references_partial: false,
    });

    let q = parse_qualified("catalog.public.orders").unwrap();
    assert_eq!(
        tools.evidence_kind(&q),
        EvidenceKind::SuccessfulReadQuery,
        "an unsupplied object touched by a query gets TOUCHED evidence"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 3: Claim supplied for X + proposal about a DIFFERENT object Y that was
// touched -> TOUCHED for Y.
// Suppression is per object, not per turn.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn supplied_claim_for_x_does_not_suppress_touched_evidence_for_y() {
    let root = temp_root("test3_per_object_suppression");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ObservationLog::new());
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
    )
    .with_supplied_objects(vec!["catalog.public.orders".to_string()]);

    // A succeeded query this turn touched `catalog.public.customers` (object Y).
    log.record(ToolObservation {
        tool: "bounded_sql_query".into(),
        outcome: ObservationOutcome::Succeeded,
        profile: None,
        objects: vec![vec!["catalog".into(), "public".into(), "customers".into()]],
        columns: Vec::new(),
        row_count: Some(1),
        truncated: None,
        references_partial: false,
    });

    let q_y = parse_qualified("catalog.public.customers").unwrap();
    assert_eq!(
        tools.evidence_kind(&q_y),
        EvidenceKind::SuccessfulReadQuery,
        "supplying claims for object X must not suppress TOUCHED evidence for a different object Y"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 4: Claim supplied for X, X never touched by a query -> UNTOUCHED.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn supplied_claim_object_not_touched_by_query_gets_untouched_evidence() {
    let root = temp_root("test4_supplied_untouched");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ObservationLog::new());
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
    )
    .with_supplied_objects(vec!["catalog.public.orders".to_string()]);

    // No query ran this turn touching `catalog.public.orders`.
    let q = parse_qualified("catalog.public.orders").unwrap();
    assert_eq!(
        tools.evidence_kind(&q),
        EvidenceKind::RepeatedObservation,
        "an object never touched by a query gets UNTOUCHED evidence"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 5: Object name matching is exact on the qualified name and case-insensitive.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn supplied_matching_is_exact_and_case_insensitive() {
    let root = temp_root("test5_case_matching");
    let store = store_at(&root.join("state.sqlite3")).await;
    let identity = profile_identity("primary");
    let log = Arc::new(ObservationLog::new());
    let tools = propose_tools(
        registry_with_primary("primary", &identity),
        store.clone(),
        log.clone(),
    )
    .with_supplied_objects(vec!["CATALOG.PUBLIC.ORDERS".to_string()]);

    // A succeeded query touched `catalog.public.orders`.
    log.record(ToolObservation {
        tool: "bounded_sql_query".into(),
        outcome: ObservationOutcome::Succeeded,
        profile: None,
        objects: vec![vec!["catalog".into(), "public".into(), "orders".into()]],
        columns: Vec::new(),
        row_count: Some(1),
        truncated: None,
        references_partial: false,
    });

    // Case variation matches: `catalog.public.orders` matches `CATALOG.PUBLIC.ORDERS`.
    let q_lower = parse_qualified("catalog.public.orders").unwrap();
    assert_eq!(
        tools.evidence_kind(&q_lower),
        EvidenceKind::RepeatedObservation,
        "case-insensitive match must suppress TOUCHED evidence"
    );

    let q_mixed = parse_qualified("Catalog.Public.Orders").unwrap();
    assert_eq!(
        tools.evidence_kind(&q_mixed),
        EvidenceKind::RepeatedObservation,
        "case-insensitive match must suppress TOUCHED evidence"
    );

    // Partial/different qualified name does NOT match: matching is exact on the qualified name.
    log.record(ToolObservation {
        tool: "bounded_sql_query".into(),
        outcome: ObservationOutcome::Succeeded,
        profile: None,
        objects: vec![vec!["other".into(), "public".into(), "orders".into()]],
        columns: Vec::new(),
        row_count: Some(1),
        truncated: None,
        references_partial: false,
    });
    let q_other = parse_qualified("other.public.orders").unwrap();
    assert_eq!(
        tools.evidence_kind(&q_other),
        EvidenceKind::SuccessfulReadQuery,
        "non-matching qualified name must not be suppressed"
    );

    let _ = fs::remove_dir_all(root);
}
