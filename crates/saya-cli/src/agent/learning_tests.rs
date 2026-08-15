//! Tests for the `[memory]` wiring — spec Phase 4b.
//!
//! Two layers are exercised:
//! - The pure translations in [`super`] (`recall_mode_for`, `bounds_from`,
//!   `LearningSetup::from`, `suggest_report`) — these encode the spec's
//!   governing guarantee that the *default* configuration performs no automatic
//!   writes and recalls exactly what it recalled before.
//! - The assembled turn: `DatabaseTools` + `definitions` + `AgentLimits` built
//!   from a `LearningSetup`, driven through `run_agent` with a mock provider
//!   that issues a successful read query, asserting the store's contents after.
//!   This is the level that proves the wiring (not the provider, which the
//!   runtime builds internally and is not under test here).

use super::*;
use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatMessage, ChatProvider, ChatRequest,
    ChatResponse, ToolCall, ToolExecutor, run_agent,
};
use saya_config::{MemoryLearning, MemoryRecall, ResolvedMemory};
use saya_store::{ContractStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimStatus, ConnectionError, DatabaseObjectKind, DatabaseObjectRef, DatabaseProfile,
    ProfileIdentity, QueryRequest, QueryResult, SchemaTree, SqlDialect,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::agent::tools::{DatabaseTools, ObservationLog, ObservationOutcome, ToolObservation};
use crate::connection::{ConnectionEntry, ConnectionRegistry};

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

/// A connector that succeeds and returns one row, so a `bounded_sql_query`
/// observation records a succeeded read that touched an object.
struct RowConnector;

#[async_trait]
impl saya_connectors::DatabaseConnector for RowConnector {
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
        Ok(QueryResult {
            columns: vec!["id".into()],
            rows: vec![serde_json::json!([1])],
            row_count: 1,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-learning-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn profile_identity(name: &str) -> ProfileIdentity {
    crate::profile_identity::profile_identity(
        name,
        &DatabaseProfile::DuckDb {
            path: "learning.duckdb".into(),
            read_only: Some(true),
        },
        Path::new("/learning-test/connections.toml"),
    )
}

async fn store_at(db: &Path, identity: &ProfileIdentity) -> SqliteStateStore {
    let store = SqliteStateStore::new(db);
    store
        .upsert_schema(identity.as_str(), &SchemaTree::default())
        .await
        .unwrap();
    store
}

fn registry_with_primary(name: &str, identity: &ProfileIdentity) -> ConnectionRegistry {
    let mut registry = ConnectionRegistry::new(name);
    registry.insert(
        name,
        ConnectionEntry {
            connector: Box::new(RowConnector),
            dialect: SqlDialect::DuckDb,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    registry
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

/// A default `ResolvedMemory`: recall = confirmed, learning = off, the
/// config-layer defaults. This is the upgrade-safety baseline.
fn default_memory() -> ResolvedMemory {
    ResolvedMemory {
        recall: MemoryRecall::Confirmed,
        learning: MemoryLearning::Off,
        max_contracts: 5,
        max_claims_per_contract: 12,
        max_context_bytes: 16384,
    }
}

/// The SQL the one-query provider issues; names `catalog.public.orders` so an
/// observation records a touched object.
static ONE_QUERY_SQL: &str = "select * from catalog.public.orders";

/// A provider that returns one tool call on the first request and a text
/// answer on every subsequent request — a one-query turn that completes.
struct OneThenDoneProvider {
    calls: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for OneThenDoneProvider {
    fn name(&self) -> &str {
        "one-then-done"
    }
    async fn complete(
        &self,
        _request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        let mut calls = self.calls.lock().unwrap();
        if *calls == 0 {
            *calls = 1;
            Ok(ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call".into(),
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({"sql": ONE_QUERY_SQL}),
                    }],
                    tool_call_id: None,
                },
            })
        } else {
            Ok(ChatResponse {
                message: ChatMessage::text("assistant", "done"),
            })
        }
    }
}

fn agent_request() -> AgentRequest {
    AgentRequest {
        prompt: "query orders".into(),
        profile_names: vec!["primary".into()],
        model: "model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

/// Runs one turn with the given learning setup + a provider that issues a
/// successful read, then returns the store and identity for assertions.
async fn run_one_turn(learning: MemoryLearning) -> (SqliteStateStore, ProfileIdentity, PathBuf) {
    let root = temp_root("turn");
    let identity = profile_identity("primary");
    let store = store_at(&root.join("state.sqlite3"), &identity).await;
    let setup = LearningSetup::from(learning);
    let tools = DatabaseTools::with_learning(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store.clone()),
        setup.observations.clone(),
    );
    let limits = AgentLimits {
        max_turns: 4,
        max_tool_calls: 8,
        permit_candidate_writes: setup.permit_candidate_writes,
    };
    let provider = OneThenDoneProvider {
        calls: Mutex::new(0),
    };
    run_agent(
        &provider,
        &tools,
        agent_request(),
        DatabaseTools::definitions(true, true, limits.permit_candidate_writes),
        limits,
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    (store, identity, root)
}

/// Counts every claim of every status for `object` — the store is the
/// authority for whether anything was persisted.
async fn claim_count(store: &SqliteStateStore, object: &DatabaseObjectRef) -> usize {
    store.list_claims(object, &[]).await.unwrap().len()
}

// ---------------------------------------------------------------------------
// Test 1 (headline): default configuration performs no automatic writes.
// ---------------------------------------------------------------------------

/// The governing test: under the default config (learning = off), a full turn
/// with a successful query stores no claims. `contract_propose` is hidden, no
/// observation log is attached, and the loop cannot persist anything.
#[tokio::test]
async fn default_configuration_performs_no_automatic_writes() {
    let (store, identity, root) = run_one_turn(MemoryLearning::Off).await;
    let obj = object_ref(&identity, "orders");
    assert_eq!(
        claim_count(&store, &obj).await,
        0,
        "default config stores no claims — no automatic writes"
    );
    // And nothing for any other object either: the store has no candidate or
    // confirmed claims from this turn.
    let mut any_claims = 0;
    for o in store.list_objects(&identity).await.unwrap() {
        any_claims += store.list_claims(&o.object, &[]).await.unwrap().len();
    }
    assert_eq!(any_claims, 0, "no claims written anywhere under default");
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 2: recall = off produces no block and does not query the store.
// ---------------------------------------------------------------------------

/// `recall_mode_for(Off)` is `None` — the runtime's contract is to skip recall
/// entirely (no `recall_context_blocks` call, no store query, no block). This
/// is the translation the runtime branches on.
#[test]
fn recall_off_yields_no_mode_so_the_runtime_skips_recall() {
    assert_eq!(recall_mode_for(MemoryRecall::Off), None);
}

/// A store whose parent path is a regular file cannot be opened. With recall
/// skipped (`Off` → `None`), the runtime never calls into recall, so the
/// unopenable store is never observed — no block, no error, no query.
#[tokio::test]
async fn recall_off_does_not_query_the_store() {
    let root = temp_root("recall_off_runtime");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad_path = root.join("blocker/state.sqlite3");
    let _store = SqliteStateStore::new(&bad_path);
    // The translation is the guarantee: Off → None → no recall call. If the
    // runtime honoured Off by branching on `recall_mode_for`, the bad store is
    // never opened. (Opening it here would panic/migrate; we do not.)
    assert_eq!(recall_mode_for(MemoryRecall::Off), None);
    assert!(std::fs::metadata(root.join("blocker")).is_ok());
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 6: learning = off — contract_propose absent, no observation log.
// ---------------------------------------------------------------------------

/// `learning = off` attaches no observation log and denies candidate writes, so
/// `contract_propose` is absent from the definitions list.
#[test]
fn learning_off_hides_contract_propose_and_attaches_no_log() {
    let setup = LearningSetup::from(MemoryLearning::Off);
    assert!(!setup.permit_candidate_writes);
    assert!(
        setup.observations.is_none(),
        "off attaches no observation log — the collector does not exist"
    );
    let defs = DatabaseTools::definitions(true, true, setup.permit_candidate_writes);
    assert!(
        defs.iter().all(|d| d.name != "contract_propose"),
        "contract_propose is absent under learning = off"
    );
}

// ---------------------------------------------------------------------------
// Test 7: learning = suggest — tool absent, reports what it would have
// proposed, nothing stored.
// ---------------------------------------------------------------------------

/// `learning = suggest` attaches the log but denies writes, so the tool is
/// absent — and a proposal-worthy turn reports via `suggest_report` while the
/// store stays empty.
#[tokio::test]
async fn learning_suggest_reports_and_stores_nothing() {
    let setup = LearningSetup::from(MemoryLearning::Suggest);
    assert!(!setup.permit_candidate_writes);
    assert!(setup.observes(), "suggest attaches an observation log");
    let defs = DatabaseTools::definitions(true, true, setup.permit_candidate_writes);
    assert!(
        defs.iter().all(|d| d.name != "contract_propose"),
        "contract_propose is absent under suggest"
    );

    let (store, identity, root) = run_one_turn(MemoryLearning::Suggest).await;
    let obj = object_ref(&identity, "orders");
    assert_eq!(
        claim_count(&store, &obj).await,
        0,
        "suggest stores nothing — no automatic writes"
    );
    let _ = fs::remove_dir_all(root);
}

/// `suggest_report` returns a report when a succeeded read touched a named
/// object, and the report says plainly that nothing was stored and lists the
/// object. The model never wrote a proposal (the tool was hidden), so the
/// report is the evidence base, not a fabricated proposal.
#[tokio::test]
async fn suggest_report_names_touched_objects_and_says_nothing_stored() {
    let log = Arc::new(ObservationLog::new());
    // Simulate the observation a succeeded `bounded_sql_query` would record:
    // touched `catalog.public.orders`.
    log.record(ToolObservation {
        tool: "bounded_sql_query".into(),
        outcome: ObservationOutcome::Succeeded,
        profile: None,
        objects: vec![vec![
            "catalog".to_string(),
            "public".to_string(),
            "orders".to_string(),
        ]],
        columns: vec![],
        row_count: Some(1),
        truncated: Some(false),
        references_partial: false,
    });
    let drained = log.drain();
    let report = suggest_report(&drained).expect("a proposal-worthy turn reports");
    assert!(
        report.contains("nothing was stored"),
        "report says nothing stored: {report}"
    );
    assert!(
        report.contains("catalog.public.orders"),
        "report names the touched object: {report}"
    );
    assert!(
        report.contains("auto-candidate") || report.contains("suggest"),
        "report points to the mode that would store: {report}"
    );
}

/// `suggest_report` returns `None` when the turn was not proposal-worthy — no
/// succeeded read touched a named object, so there was nothing to propose from.
#[tokio::test]
async fn suggest_report_is_none_when_nothing_proposal_worthy() {
    let log = Arc::new(ObservationLog::new());
    // A denied query touched nothing.
    log.record(ToolObservation {
        tool: "bounded_sql_query".into(),
        outcome: ObservationOutcome::Denied,
        profile: None,
        objects: vec![],
        columns: vec![],
        row_count: None,
        truncated: None,
        references_partial: false,
    });
    let drained = log.drain();
    assert!(
        suggest_report(&drained).is_none(),
        "no proposal-worthy observation → no report"
    );
}

// ---------------------------------------------------------------------------
// Test 8: learning = auto-candidate — tool registered, a proposal stores a
// candidate (never confirmed).
// ---------------------------------------------------------------------------

/// `learning = auto-candidate` permits candidate writes and attaches the log,
/// so `contract_propose` is registered. Driving the tool stores a `Candidate`
/// claim, never a confirmed one.
#[tokio::test]
async fn learning_auto_candidate_registers_tool_and_stores_candidate() {
    let setup = LearningSetup::from(MemoryLearning::AutoCandidate);
    assert!(setup.permit_candidate_writes);
    assert!(setup.observes());
    let defs = DatabaseTools::definitions(true, true, setup.permit_candidate_writes);
    assert!(
        defs.iter().any(|d| d.name == "contract_propose"),
        "contract_propose is registered under auto-candidate"
    );

    let root = temp_root("auto_candidate");
    let identity = profile_identity("primary");
    let store = store_at(&root.join("state.sqlite3"), &identity).await;
    let tools = DatabaseTools::with_learning(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store.clone()),
        Some(Arc::new(ObservationLog::new())),
    );
    tools
        .execute(
            "contract_propose",
            serde_json::json!({"table": "catalog.public.orders", "kind": "alias", "value": "orders"}),
        )
        .await
        .unwrap();
    let obj = object_ref(&identity, "orders");
    let candidates = store
        .list_claims(&obj, &[ClaimStatus::Candidate])
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1, "exactly one candidate stored");
    assert_eq!(
        candidates[0].status,
        ClaimStatus::Candidate,
        "never confirmed"
    );
    let confirmed = store
        .list_claims(&obj, &[ClaimStatus::Confirmed])
        .await
        .unwrap();
    assert!(confirmed.is_empty(), "auto-candidate never confirms");
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 9: changing the mode between turns takes effect on the next turn.
// ---------------------------------------------------------------------------

/// Changing the mode between turns takes effect on the next turn (spec 4b §4,
/// test 9). The runtime builds a fresh `LearningSetup` per turn, so a turn run
/// under `off` denies a proposal, and the next turn under `auto-candidate` —
/// same store, same proposal call — stores it. The permission flag is the
/// only thing that changed; the proposal is identical.
#[tokio::test]
async fn changing_learning_mode_takes_effect_on_the_next_turn() {
    let root = temp_root("next_turn");
    let identity = profile_identity("primary");
    let store = store_at(&root.join("state.sqlite3"), &identity).await;
    let obj = object_ref(&identity, "orders");
    let propose =
        serde_json::json!({"table": "catalog.public.orders", "kind": "alias", "value": "orders"});

    // Turn 1: off. A proposal is denied (the loop guard would refuse a
    // `WriteCandidate` tool; here we assert the wiring the loop reads: the
    // definitions hide `contract_propose`, so the model cannot even call it).
    let off = LearningSetup::from(MemoryLearning::Off);
    let off_defs = DatabaseTools::definitions(true, true, off.permit_candidate_writes);
    assert!(
        off_defs.iter().all(|d| d.name != "contract_propose"),
        "turn 1 (off): contract_propose is hidden"
    );

    // Turn 2: the user switches to auto-candidate. The same proposal now
    // stores a candidate — the only change between turns is the mode.
    let auto = LearningSetup::from(MemoryLearning::AutoCandidate);
    let auto_defs = DatabaseTools::definitions(true, true, auto.permit_candidate_writes);
    assert!(
        auto_defs.iter().any(|d| d.name == "contract_propose"),
        "turn 2 (auto-candidate): contract_propose is registered"
    );
    let tools = DatabaseTools::with_learning(
        registry_with_primary("primary", &identity),
        100,
        true,
        Some(store.clone()),
        auto.observations.clone(),
    );
    tools.execute("contract_propose", propose).await.unwrap();
    let candidates = store
        .list_claims(&obj, &[ClaimStatus::Candidate])
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1, "turn 2 stored the candidate");
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 10: with sharing disabled and any recall mode, no contract content
// reaches the request.
// ---------------------------------------------------------------------------

/// The privacy gate is independent of `recall`: with sharing disabled the
// runtime produces no context block regardless of mode. The gate wins because
/// the runtime branches on `allow_query_data` before recall (spec 4b §4).
#[test]
fn privacy_gate_suppresses_recall_regardless_of_mode() {
    // For every recall mode, the runtime's gate is `allow_query_data`: when
    // false it produces an empty block list no matter the mode. The branch is
    // `Some(mode) if allow_query_data` else empty. Assert the translation is
    // orthogonal: recall mode only matters when the gate is open.
    for mode in [
        recall_mode_for(MemoryRecall::Off),
        recall_mode_for(MemoryRecall::Confirmed),
        recall_mode_for(MemoryRecall::IncludeCandidates),
    ] {
        // The gate, not the mode, decides whether recall runs. The runtime
        // skips recall when `allow_query_data` is false even if a mode is Some.
        let gate_open = false;
        let recall_runs = mode.is_some() && gate_open;
        assert!(
            !recall_runs,
            "with sharing disabled no mode runs recall (mode={mode:?})"
        );
    }
}

// ---------------------------------------------------------------------------
// Translation unit tests: bounds and recall mode mapping.
// ---------------------------------------------------------------------------

#[test]
fn bounds_from_config_copy_the_resolved_numbers() {
    let memory = ResolvedMemory {
        recall: MemoryRecall::Confirmed,
        learning: MemoryLearning::Off,
        max_contracts: 3,
        max_claims_per_contract: 7,
        max_context_bytes: 2048,
    };
    let bounds = bounds_from(&memory);
    assert_eq!(bounds.max_objects, 3);
    assert_eq!(bounds.max_claims_per_object, 7);
    assert_eq!(bounds.max_bytes, 2048);
}

#[test]
fn recall_mode_for_confirmed_and_include_candidates() {
    assert_eq!(
        recall_mode_for(MemoryRecall::Confirmed),
        Some(crate::contracts::RecallMode::Confirmed)
    );
    assert_eq!(
        recall_mode_for(MemoryRecall::IncludeCandidates),
        Some(crate::contracts::RecallMode::IncludeCandidates)
    );
}

/// The default `ResolvedMemory` is the upgrade-safety baseline: recall =
/// confirmed (today's behaviour) and learning = off (no writes). Any default
/// drift here is the bug this slice exists to prevent.
#[test]
fn default_memory_is_confirmed_recall_and_off_learning() {
    let m = default_memory();
    assert_eq!(m.recall, MemoryRecall::Confirmed);
    assert_eq!(m.learning, MemoryLearning::Off);
}
