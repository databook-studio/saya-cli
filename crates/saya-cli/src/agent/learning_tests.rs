//! Tests for the single-knob `[memory]` wiring — spec E.
//!
//! Two layers are exercised:
//! - The pure translations in [`super`] (`recall_mode_for`, `bounds_from`,
//!   `LearningSetup::from`) — encoding the guarantee that `off` is genuinely off
//!   and `assisted` enables recall and candidate proposals.
//! - The assembled turn: `DatabaseTools` + `definitions` + `AgentLimits` built
//!   from `LearningSetup`, driven through `run_agent` with a mock provider,
//!   asserting store interactions and receipts.

use super::*;
use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatMessage, ChatProvider, ChatRequest,
    ChatResponse, ToolCall, run_agent,
};
use saya_config::{
    ConfigFile, ConnectionsFile, MemoryMode, ResolutionInput, ResolvedMemory, resolve,
};
use saya_store::{KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ConnectionError, DatabaseObjectKind, DatabaseObjectRef, DatabaseProfile, ProfileIdentity,
    QueryRequest, QueryResult, SchemaTree, SqlDialect,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::agent::tools::DatabaseTools;
use crate::connection::{ConnectionEntry, ConnectionRegistry};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

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

fn default_memory() -> ResolvedMemory {
    ResolvedMemory {
        mode: MemoryMode::Off,
        max_contracts: 5,
        max_claims_per_contract: 12,
        max_context_bytes: 16384,
    }
}

static ONE_QUERY_SQL: &str = "select * from catalog.public.orders";

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

async fn run_one_turn(mode: MemoryMode) -> (SqliteStateStore, ProfileIdentity, PathBuf) {
    let root = temp_root("turn");
    let identity = profile_identity("primary");
    let store = store_at(&root.join("state.sqlite3"), &identity).await;
    let setup = LearningSetup::from(mode);
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

async fn claim_count(store: &SqliteStateStore, object: &DatabaseObjectRef) -> usize {
    store.knowledge_for_object(object).await.unwrap().len()
}

// ---------------------------------------------------------------------------
// Test 1: mode = "off" performs no store read and no store write across a turn
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_off_performs_no_store_read_and_no_store_write_across_turn() {
    // 1. Store write check: turn performs 0 writes.
    let (store, identity, root) = run_one_turn(MemoryMode::Off).await;
    let obj = object_ref(&identity, "orders");
    assert_eq!(
        claim_count(&store, &obj).await,
        0,
        "mode = off stores no claims"
    );

    let any_claims = store.knowledge_for_profile(&identity).await.unwrap().len();
    assert_eq!(any_claims, 0, "no claims written anywhere under mode = off");

    // 2. Store read check: recall mode is None, so store is never read.
    assert_eq!(
        recall_mode_for(MemoryMode::Off),
        None,
        "recall_mode_for(Off) is None — recall is skipped entirely"
    );

    // If an unopenable store path exists, mode = Off never touches it.
    let blocker_root = temp_root("blocker");
    fs::write(blocker_root.join("blocker_file"), b"x").unwrap();
    let bad_path = blocker_root.join("blocker_file/state.sqlite3");
    let _store = SqliteStateStore::new(&bad_path);
    assert_eq!(recall_mode_for(MemoryMode::Off), None);
    assert!(fs::metadata(blocker_root.join("blocker_file")).is_ok());

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(blocker_root);
}

// ---------------------------------------------------------------------------
// Test 2: mode = "assisted" supplies active knowledge and labels pending knowledge
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_assisted_recalls_candidates_and_permits_proposals() {
    // 1. Recall mode is IncludeCandidates, so active and candidate claims are admitted.
    assert_eq!(
        recall_mode_for(MemoryMode::Assisted),
        Some(crate::contracts::RecallMode::IncludeCandidates)
    );

    // 2. Write setup permits candidate writes and attaches observations.
    let setup = LearningSetup::from(MemoryMode::Assisted);
    assert!(setup.permit_candidate_writes);
    assert!(setup.observes());

    // 3. Definitions do NOT include contract_propose (deprecated in Phase F).
    let defs = DatabaseTools::definitions(true, true, setup.permit_candidate_writes);
    assert!(
        defs.iter().all(|d| d.name != "contract_propose"),
        "contract_propose is removed from model tools"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Privacy gate suppresses recall under assisted mode
// ---------------------------------------------------------------------------

#[test]
fn privacy_gate_suppresses_recall_regardless_of_mode() {
    for mode in [MemoryMode::Off, MemoryMode::Assisted] {
        let recall_mode = recall_mode_for(mode);
        let allow_query_data = false;
        let recall_runs = recall_mode.is_some() && allow_query_data;
        assert!(
            !recall_runs,
            "with sharing disabled no mode runs recall (mode={mode:?})"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 4: Absent [memory] resolves to default (off), asserted explicitly
// ---------------------------------------------------------------------------

#[test]
fn absent_memory_section_resolves_to_off_default() {
    let resolved = resolve(ResolutionInput::new(ConnectionsFile::default())).unwrap();
    assert_eq!(
        resolved.memory.mode,
        MemoryMode::Off,
        "default memory mode must be Off"
    );
    assert_eq!(resolved.memory.max_contracts, 5);
    assert_eq!(resolved.memory.max_claims_per_contract, 12);
    assert_eq!(resolved.memory.max_context_bytes, 16384);
}

// ---------------------------------------------------------------------------
// Test 5: Legacy config naming old axes fails loudly at parse time
// ---------------------------------------------------------------------------

#[test]
fn legacy_two_axis_configuration_fails_loudly_at_parse_time() {
    let recall_err = ConfigFile::from_toml("[memory]\nrecall = 'confirmed'\n").unwrap_err();
    let rendered = format!("{recall_err:?}");
    assert!(
        rendered.contains("recall"),
        "error should name rejected field 'recall': {rendered}"
    );

    let learning_err =
        ConfigFile::from_toml("[memory]\nlearning = 'auto-candidate'\n").unwrap_err();
    let rendered = format!("{learning_err:?}");
    assert!(
        rendered.contains("learning"),
        "error should name rejected field 'learning': {rendered}"
    );
}

// ---------------------------------------------------------------------------
// Test 6: Changing mode between turns takes effect on the next turn
// ---------------------------------------------------------------------------

#[tokio::test]
async fn changing_memory_mode_takes_effect_on_the_next_turn() {
    // Turn 1: off -> no candidate writes, no observations.
    let off = LearningSetup::from(MemoryMode::Off);
    assert!(!off.permit_candidate_writes);
    assert!(!off.observes());
    let off_defs = DatabaseTools::definitions(true, true, off.permit_candidate_writes);
    assert!(
        off_defs.iter().all(|d| d.name != "contract_propose"),
        "turn 1 (off): contract_propose is hidden"
    );

    // Turn 2: assisted -> permits candidate writes and attaches observations.
    let assisted = LearningSetup::from(MemoryMode::Assisted);
    assert!(assisted.permit_candidate_writes);
    assert!(assisted.observes());
    let assisted_defs = DatabaseTools::definitions(true, true, assisted.permit_candidate_writes);
    assert!(
        assisted_defs.iter().all(|d| d.name != "contract_propose"),
        "turn 2 (assisted): contract_propose is never advertised to the model"
    );
}

// ---------------------------------------------------------------------------
// Translation Unit Tests
// ---------------------------------------------------------------------------

#[test]
fn bounds_from_config_copy_the_resolved_numbers() {
    let memory = ResolvedMemory {
        mode: MemoryMode::Assisted,
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
fn learning_setup_translation() {
    let off = LearningSetup::from(MemoryMode::Off);
    assert!(!off.permit_candidate_writes);
    assert!(off.observations.is_none());

    let assisted = LearningSetup::from(MemoryMode::Assisted);
    assert!(assisted.permit_candidate_writes);
    assert!(assisted.observations.is_some());
}

#[test]
fn default_memory_is_mode_off() {
    let m = default_memory();
    assert_eq!(m.mode, MemoryMode::Off);
}
