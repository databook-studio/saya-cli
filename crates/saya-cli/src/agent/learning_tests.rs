//! Tests for the single-knob `[memory]` wiring.
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
            Ok(ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({"sql": ONE_QUERY_SQL}),
                }],
                tool_call_id: None,
            }))
        } else {
            Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
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
        max_turns: Some(4),
        max_tool_calls: Some(8),
        permit_candidate_writes: setup.permit_candidate_writes,
        ..saya_agent::AgentLimits::default()
    };
    let provider = OneThenDoneProvider {
        calls: Mutex::new(0),
    };
    run_agent(
        &provider,
        &tools,
        agent_request(),
        DatabaseTools::definitions(true, true, limits.permit_candidate_writes, false, true),
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

include!("learning_tests/mode_behavior.rs");

// ---------------------------------------------------------------------------
// Translation Unit Tests
// ---------------------------------------------------------------------------

include!("learning_tests/translation.rs");
