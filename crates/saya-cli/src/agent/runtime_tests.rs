//! Tests for the `KnowledgeSupplied` event — spec P1b.
//!
//! Two layers:
//! - **The pure mapping** [`crate::agent::knowledge_event::knowledge_supplied_event`] — the
//!   three-state decision (Off / Skipped / Ran) and the claim mapping, unit-tested
//!   without a live database (tests 3, 4, 5, 7, 8 and the content of test 1).
//! - **The assembled turn** via [`super::run_prompt_with_inputs`] with an
//!   injected mock provider and an idle registry — proves the emit is before
//!   the provider request (test 2), that exactly one event names the supplied
//!   claims (test 1), and that a store failure still emits and still runs the
//!   turn (test 6). The provider and registry are injected because
//!   `run_prompt_with_sink` builds a live provider and a live (connecting)
//!   registry from config, which a unit test cannot supply; the inner
//!   `run_prompt_with_inputs` is the seam.

use super::super::knowledge_event::knowledge_supplied_event;
use super::super::turn_inputs::TurnInputs;
use super::*;
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    KnowledgeOutcome, ProposedClaimDto, ProviderError, SuppliedClaimDto, SuppliedContractDto,
    ToolCall,
};
use saya_config::{
    AiProvider, ColorChoice, MemoryLearning, MemoryRecall, OutputFormat, ResolvedAi,
    ResolvedConfig, ResolvedMemory,
};
use saya_store::{ContractStore, ProposeClaim, ProposeOutcome, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, Column, ConnectionError, Database,
    DatabaseObjectKind, DatabaseObjectRef, DatabaseProfile, ProfileIdentity, QueryRequest,
    QueryResult, Schema, SchemaTree, SqlDialect, Table,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::config::runtime::RuntimeConfig;
use crate::contracts::{RecallOutcomeKind, RecallReceipt};

// ---------------------------------------------------------------------------
// shared harness
// ---------------------------------------------------------------------------

/// A connector that never touches a live database: recall reads the store, not
/// the connector, so the registry only needs an entry to carry an identity.
struct IdleConnector;

#[async_trait]
impl saya_connectors::DatabaseConnector for IdleConnector {
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
        "saya-runtime-p1b-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn identity_for(name: &str) -> ProfileIdentity {
    crate::profile_identity::profile_identity(
        name,
        &DatabaseProfile::DuckDb {
            path: "runtime-p1b.duckdb".into(),
            read_only: Some(true),
        },
        Path::new("/runtime-p1b-test/connections.toml"),
    )
}

fn registry_for(name: &str, identity: &ProfileIdentity) -> ConnectionRegistry {
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

async fn store_at(db: &Path, identity: &ProfileIdentity) -> SqliteStateStore {
    let store = SqliteStateStore::new(db);
    store
        .upsert_schema(identity.as_str(), &SchemaTree::default())
        .await
        .unwrap();
    store
}

fn object(identity: &ProfileIdentity, name: &str) -> DatabaseObjectRef {
    DatabaseObjectRef::new(
        identity.clone(),
        "catalog",
        "public",
        name,
        DatabaseObjectKind::Table,
    )
    .unwrap()
}

fn orders_table() -> Table {
    Table {
        name: "orders".into(),
        columns: vec![
            Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            },
            Column {
                name: "created_at".into(),
                data_type: "timestamp".into(),
                nullable: false,
            },
        ],
    }
}

fn live_fingerprint(tree: &Table) -> saya_types::SchemaFingerprint {
    saya_types::SchemaFingerprint::of_table(DatabaseObjectKind::Table, tree)
}

fn orders_schema(identity: &ProfileIdentity) -> (ProfileIdentity, SchemaTree) {
    let table = orders_table();
    let tree = SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![table],
            }],
        }],
    };
    (identity.clone(), tree)
}

async fn remember_default_time_column(
    store: &SqliteStateStore,
    obj: &DatabaseObjectRef,
    fingerprint: &saya_types::SchemaFingerprint,
    column: &str,
    status: ClaimStatus,
    origin: ClaimOrigin,
) -> ClaimId {
    let payload = ClaimPayload::default_time_column(column).unwrap();
    let request = ProposeClaim {
        object: obj.clone(),
        fingerprint: fingerprint.clone(),
        referenced_columns: payload.referenced_column_name_snapshots(),
        payload,
        origin,
        initial_status: status,
        evidence: None,
    };
    match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    }
}

/// A minimal `RuntimeConfig` carrying only what `run_prompt_with_inputs` reads
/// (`resolved.memory`, `max_rows`, `max_iterations`). The provider and registry
/// are injected via `TurnInputs`, so the live-build fields are never used.
fn test_runtime(memory: ResolvedMemory) -> RuntimeConfig {
    RuntimeConfig {
        resolved: ResolvedConfig {
            profile_name: None,
            profile: None,
            ai: ResolvedAi {
                provider: AiProvider::Ollama,
                model: "test-model".into(),
                base_url: None,
                api_key: None,
                allow_data_sharing: true,
                temperature: 0.0,
            },
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            query_timeout_seconds: 5,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Auto,
            memory,
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-runtime-p1b"),
        secret_values: BTreeMap::new(),
    }
}

fn default_memory() -> ResolvedMemory {
    ResolvedMemory {
        recall: MemoryRecall::Confirmed,
        learning: MemoryLearning::Off,
        max_contracts: 5,
        max_claims_per_contract: 12,
        max_context_bytes: 16384,
    }
}

/// `auto-candidate` memory: the learning mode that permits candidate writes, so
/// `contract_propose` is registered and the loop will execute it (spec 4b §2).
fn auto_candidate_memory() -> ResolvedMemory {
    ResolvedMemory {
        recall: MemoryRecall::Off,
        learning: MemoryLearning::AutoCandidate,
        max_contracts: 5,
        max_claims_per_contract: 12,
        max_context_bytes: 16384,
    }
}

/// A sink that records every event in order. `knowledge_log`, when shared with
/// the provider, is the ordering oracle: the sink pushes `"knowledge"` when it
/// sees `KnowledgeSupplied`, the provider pushes `"provider"` when it is
/// called, and the order in the log proves the emit preceded the request.
struct RecordingSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
    knowledge_log: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        if matches!(event, AgentEvent::KnowledgeSupplied { .. }) {
            self.knowledge_log.lock().unwrap().push("knowledge");
        }
        self.events.lock().unwrap().push(event);
    }
}

/// A provider that returns one text answer, recording `"provider"` into the
/// shared log at call time so the ordering test can prove the emit came first.
struct AnswerProvider {
    answer: &'static str,
    log: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl ChatProvider for AnswerProvider {
    fn name(&self) -> &str {
        "answer"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.log.lock().unwrap().push("provider");
        Ok(ChatResponse {
            message: ChatMessage::text("assistant", self.answer),
        })
    }
}

// ===========================================================================
// Test 1: a turn that supplies claims emits exactly one KnowledgeSupplied,
// naming those claims.
// ===========================================================================

#[tokio::test]
async fn a_turn_supplying_claims_emits_one_event_naming_those_claims() {
    let root = temp_root("t1_supply");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    // Seed a confirmed default_time_column claim under a matching live schema.
    let obj = object(&identity, "orders");
    let tree = orders_schema(&identity);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&orders_table());
    let claim_id = remember_default_time_column(
        &store,
        &obj,
        &fp,
        "created_at",
        ClaimStatus::Confirmed,
        ClaimOrigin::UserExplicit,
    )
    .await;

    let events = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: log.clone(),
    };
    let provider = AnswerProvider {
        answer: "done",
        log: log.clone(),
    };
    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
        },
        provider: Box::new(provider),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(default_memory());
    run_prompt_with_inputs(
        &runtime,
        inputs,
        "orders by month",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        Some(store),
        None,
        None,
    )
    .await
    .unwrap();

    let captured = events.lock().unwrap();
    // Exactly one KnowledgeSupplied.
    let count = captured
        .iter()
        .filter(|e| matches!(e, AgentEvent::KnowledgeSupplied { .. }))
        .count();
    assert_eq!(count, 1, "exactly one KnowledgeSupplied: {captured:?}");
    // It names the supplied claim.
    let event = captured
        .iter()
        .find_map(|e| match e {
            AgentEvent::KnowledgeSupplied { contracts, .. } => Some(contracts),
            _ => None,
        })
        .expect("KnowledgeSupplied present");
    assert_eq!(event.len(), 1, "one object supplied: {event:?}");
    assert_eq!(event[0].object, "catalog.public.orders");
    assert_eq!(event[0].profile, "analytics", "name, not identity");
    assert_eq!(event[0].claims.len(), 1);
    assert_eq!(event[0].claims[0].claim_id, claim_id);
    assert_eq!(event[0].claims[0].value, "created_at");
    assert_eq!(event[0].claims[0].status, ClaimStatus::Confirmed);
    let _ = fs::remove_dir_all(root);
}

// ===========================================================================
// Test 2: the event is emitted BEFORE any provider request. Asserted against
// the sink's event sequence AND a shared log the provider writes at call time,
// not merely that the event appears (the bug this slice prevents is emitting
// after the answer).
// ===========================================================================

#[tokio::test]
async fn knowledge_supplied_precedes_the_provider_request() {
    let root = temp_root("t2_order");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let obj = object(&identity, "orders");
    let tree = orders_schema(&identity);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&orders_table());
    remember_default_time_column(
        &store,
        &obj,
        &fp,
        "created_at",
        ClaimStatus::Confirmed,
        ClaimOrigin::UserExplicit,
    )
    .await;

    let events = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: log.clone(),
    };
    let provider = AnswerProvider {
        answer: "done",
        log: log.clone(),
    };
    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
        },
        provider: Box::new(provider),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(default_memory());
    run_prompt_with_inputs(
        &runtime,
        inputs,
        "orders by month",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        Some(store),
        None,
        None,
    )
    .await
    .unwrap();

    // The shared log records the order of the two events: the sink pushed
    // "knowledge" on KnowledgeSupplied, then the provider pushed "provider"
    // when `complete` was called. "knowledge" first proves the emit preceded
    // the provider request — the point of the slice.
    let log = log.lock().unwrap();
    assert_eq!(
        log.first(),
        Some(&"knowledge"),
        "KnowledgeSupplied was emitted before the provider was called: {log:?}"
    );
    assert!(
        log.contains(&"provider"),
        "the provider was called after the emit: {log:?}"
    );
    let knowledge_idx = log
        .iter()
        .position(|e| *e == "knowledge")
        .expect("knowledge present");
    let provider_idx = log
        .iter()
        .position(|e| *e == "provider")
        .expect("provider present");
    assert!(
        knowledge_idx < provider_idx,
        "emit index {knowledge_idx} must precede provider index {provider_idx}: {log:?}"
    );

    // And in the sink's own event sequence, KnowledgeSupplied precedes the
    // first loop event (AssistantText from the provider's answer).
    let captured = events.lock().unwrap();
    let knowledge_event_idx = captured
        .iter()
        .position(|e| matches!(e, AgentEvent::KnowledgeSupplied { .. }))
        .expect("KnowledgeSupplied in sink");
    let first_loop_idx = captured
        .iter()
        .position(|e| {
            matches!(
                e,
                AgentEvent::AssistantText { .. }
                    | AgentEvent::ToolRequested { .. }
                    | AgentEvent::Complete
            )
        })
        .expect("a loop event after the provider call");
    assert!(
        knowledge_event_idx < first_loop_idx,
        "KnowledgeSupplied (idx {knowledge_event_idx}) must precede the first loop event (idx {first_loop_idx}): {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}

// ===========================================================================
// Tests 3 & 4: the three-state decision (Off / Skipped / Ran), as pure tests
// on `knowledge_supplied_event`. The runtime calls this; the three states are
// distinguishable here without a live database.
// ===========================================================================

#[test]
fn recall_off_emits_the_off_outcome() {
    // recall = off: SAYA did not look. Distinct from the privacy gate.
    let receipt = RecallReceipt::configured_off();
    assert_eq!(receipt.kind, RecallOutcomeKind::ConfiguredOff);
    let event = knowledge_supplied_event(&receipt);
    assert!(matches!(
        event,
        AgentEvent::KnowledgeSupplied {
            outcome: KnowledgeOutcome::Off,
            contracts,
            dropped_by_bounds: 0,
        } if contracts.is_empty()
    ));
}

#[test]
fn privacy_gate_closed_emits_the_skipped_outcome() {
    // Privacy gate closed: SAYA was not allowed to look. No store query.
    let receipt = RecallReceipt::privacy_gate_closed();
    assert_eq!(receipt.kind, RecallOutcomeKind::PrivacyGateClosed);
    let event = knowledge_supplied_event(&receipt);
    assert!(matches!(
        event,
        AgentEvent::KnowledgeSupplied {
            outcome: KnowledgeOutcome::Skipped,
            contracts,
            dropped_by_bounds: 0,
        } if contracts.is_empty()
    ));
}

#[test]
fn recall_ran_and_found_nothing_emits_ran_distinct_from_off_and_skipped() {
    // Recall ran and matched nothing: a Ran receipt with empty supplied. This
    // is the third state — distinct from Off (did not look) and Skipped (not
    // allowed to look). The three outcomes must not collapse.
    let receipt = RecallReceipt::ran_empty(false);
    let event = knowledge_supplied_event(&receipt);
    assert!(matches!(
        event,
        AgentEvent::KnowledgeSupplied {
            outcome: KnowledgeOutcome::Ran { store_unavailable: false },
            contracts,
            dropped_by_bounds: 0,
        } if contracts.is_empty()
    ));
    // The three are distinguishable.
    let off = knowledge_supplied_event(&RecallReceipt::configured_off());
    let skipped = knowledge_supplied_event(&RecallReceipt::privacy_gate_closed());
    let ran = knowledge_supplied_event(&receipt);
    assert_ne!(outcome_of(&off), outcome_of(&skipped));
    assert_ne!(outcome_of(&skipped), outcome_of(&ran));
    assert_ne!(outcome_of(&off), outcome_of(&ran));
}

fn outcome_of(event: &AgentEvent) -> KnowledgeOutcome {
    match event {
        AgentEvent::KnowledgeSupplied { outcome, .. } => *outcome,
        _ => panic!("not a KnowledgeSupplied event"),
    }
}

// ===========================================================================
// Test 5: a candidate claim's status survives into the event, distinct from
// confirmed.
// ===========================================================================

#[test]
fn a_candidate_claims_status_survives_into_the_event() {
    let claim_id = ClaimId::parse("c-candidate-001").unwrap();
    let receipt = RecallReceipt {
        kind: RecallOutcomeKind::Ran {
            store_unavailable: false,
        },
        supplied: vec![crate::contracts::SuppliedContract {
            profile: "analytics".into(),
            object: "catalog.public.orders".into(),
            schema_state: "current",
            claims: vec![crate::contracts::SuppliedClaim {
                claim_id: claim_id.clone(),
                kind: "default_time_column",
                value: "created_at".into(),
                column: Some("created_at".into()),
                status: ClaimStatus::Candidate,
            }],
        }],
        dropped_by_bounds: 0,
    };
    let event = knowledge_supplied_event(&receipt);
    let contracts = match event {
        AgentEvent::KnowledgeSupplied { contracts, .. } => contracts,
        _ => panic!("expected KnowledgeSupplied"),
    };
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].claims.len(), 1);
    assert_eq!(contracts[0].claims[0].claim_id, claim_id);
    // The candidate status survives as Candidate, not flattened to confirmed.
    assert_eq!(contracts[0].claims[0].status, ClaimStatus::Candidate);
    // Distinct from a confirmed claim's status.
    assert_ne!(contracts[0].claims[0].status, ClaimStatus::Confirmed);
}

// ===========================================================================
// Test 6: store unavailable still runs the turn and still emits (a Ran event
// with store_unavailable: true). Driven through the runtime with an
// unopenable store so recall fails soft.
// ===========================================================================

#[tokio::test]
async fn store_unavailable_still_runs_the_turn_and_emits() {
    let root = temp_root("t6_store_unavailable");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad_path = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad_path); // parent is a file → unopenable
    let identity = identity_for("analytics");

    let events = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: log.clone(),
    };
    let provider = AnswerProvider {
        answer: "done anyway",
        log: log.clone(),
    };
    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
        },
        provider: Box::new(provider),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(default_memory());
    let result = run_prompt_with_inputs(
        &runtime,
        inputs,
        "orders by month",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        Some(store),
        None,
        None,
    )
    .await;

    // The turn still completes despite the store failure (recall is fail-soft).
    assert!(
        result.is_ok(),
        "turn completes despite store failure: {result:?}"
    );
    let captured = events.lock().unwrap();
    // The event records the failure as Ran { store_unavailable: true }, not as
    // an error and not as silence.
    let event = captured
        .iter()
        .find_map(|e| match e {
            AgentEvent::KnowledgeSupplied { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .expect("KnowledgeSupplied emitted despite store failure");
    assert_eq!(
        event,
        KnowledgeOutcome::Ran {
            store_unavailable: true
        },
        "store failure is Ran{{store_unavailable: true}}, not silence: {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}

// ===========================================================================
// Test 7: no opaque ProfileIdentity value appears in a serialized event.
// Asserted directly on the serialized JSON — the identity is a hash over
// connection material and must not reach output.
// ===========================================================================

#[test]
fn no_opaque_profile_identity_value_appears_in_the_event() {
    let identity = identity_for("analytics");
    let receipt = RecallReceipt {
        kind: RecallOutcomeKind::Ran {
            store_unavailable: false,
        },
        supplied: vec![crate::contracts::SuppliedContract {
            profile: "analytics".into(),
            object: "catalog.public.orders".into(),
            schema_state: "current",
            claims: vec![crate::contracts::SuppliedClaim {
                claim_id: ClaimId::parse("c-abc123").unwrap(),
                kind: "default_time_column",
                value: "created_at".into(),
                column: None,
                status: ClaimStatus::Confirmed,
            }],
        }],
        dropped_by_bounds: 0,
    };
    let event = knowledge_supplied_event(&receipt);
    let json = serde_json::to_string(&event).expect("serializes");
    // The opaque identity string appears nowhere in the serialized event.
    assert!(
        !json.contains(identity.as_str()),
        "opaque identity leaked into the serialized event: {json}"
    );
    // The human-facing name does appear, in place of the identity.
    assert!(
        json.contains("analytics"),
        "the profile name (not the identity) is what the event carries: {json}"
    );
}

// ===========================================================================
// Test 8: the event round-trips through serde with its `type` tag.
// ===========================================================================

#[test]
fn knowledge_supplied_round_trips_through_serde_with_type_tag() {
    let event = AgentEvent::knowledge_supplied(
        KnowledgeOutcome::Ran {
            store_unavailable: true,
        },
        vec![SuppliedContractDto {
            profile: "analytics".into(),
            object: "catalog.public.orders".into(),
            schema_state: "needs_review".into(),
            claims: vec![SuppliedClaimDto {
                claim_id: ClaimId::parse("c-roundtrip").unwrap(),
                kind: "default_time_column".into(),
                value: "created_at".into(),
                column: Some("created_at".into()),
                status: ClaimStatus::Candidate,
            }],
        }],
        3,
    );
    let json = serde_json::to_string(&event).expect("serializes");
    // The outer tag is `knowledge_supplied` (snake_case, per AgentEvent's
    // `#[serde(tag = "type", rename_all = "snake_case")]`).
    assert!(
        json.contains(r#""type":"knowledge_supplied""#),
        "carries the type tag: {json}"
    );
    let back: AgentEvent = serde_json::from_str(&json).expect("deserializes back");
    assert_eq!(back, event, "round-trips with the claims and status intact");

    // The three outcomes each round-trip too.
    for (outcome, expected) in [
        (KnowledgeOutcome::Off, r#""off""#),
        (KnowledgeOutcome::Skipped, r#""skipped""#),
        (
            KnowledgeOutcome::Ran {
                store_unavailable: false,
            },
            r#"{"ran":{"store_unavailable":false}}"#,
        ),
    ] {
        let text = serde_json::to_string(&outcome).expect("serializes");
        assert_eq!(text, expected, "outcome {outcome:?} serializes as expected");
        let back: KnowledgeOutcome = serde_json::from_str(&text).expect("deserializes back");
        assert_eq!(back, outcome, "outcome {outcome:?} round-trips");
    }
}

// ===========================================================================
// P2d — a persisted proposal emits one KnowledgeProposed through the runtime.
//
// The tool-level tests (propose_tools_tests) assert the data the event would
// carry by draining the `ProposedClaimsLog` directly; this test proves the
// runtime actually emits the event — once per persisted claim, after the loop
// drains the log, naming what was stored (spec P2d §2/§5.1).
// ===========================================================================

/// A provider that issues one `contract_propose` call on the first request and
/// a text answer on every subsequent one — a one-proposal turn that completes.
struct ProposeProvider {
    calls: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for ProposeProvider {
    fn name(&self) -> &str {
        "propose-once"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut calls = self.calls.lock().unwrap();
        if *calls == 0 {
            *calls = 1;
            Ok(ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call".into(),
                        name: "contract_propose".into(),
                        arguments: serde_json::json!({
                            "table": "catalog.public.orders",
                            "kind": "alias",
                            "value": "orders",
                        }),
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

/// A turn that persists one proposal emits exactly one `KnowledgeProposed`,
/// naming the claim that was stored (spec P2d §5.1).
#[tokio::test]
async fn a_turn_persisting_a_proposal_emits_one_knowledge_proposed() {
    let root = temp_root("p2d_emit");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
        },
        provider: Box::new(ProposeProvider {
            calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(auto_candidate_memory());
    run_prompt_with_inputs(
        &runtime,
        inputs,
        "remember the orders alias",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        Some(store.clone()),
        None,
        None,
    )
    .await
    .expect("turn completes");

    let claim_id = {
        let captured = events.lock().unwrap();
        let proposed: Vec<&ProposedClaimDto> = captured
            .iter()
            .filter_map(|event| match event {
                AgentEvent::KnowledgeProposed { claim } => Some(claim),
                _ => None,
            })
            .collect();
        assert_eq!(
            proposed.len(),
            1,
            "exactly one KnowledgeProposed: {captured:?}"
        );
        let claim = proposed[0];
        assert_eq!(
            claim.profile, "analytics",
            "the profile name, not the identity"
        );
        assert_eq!(claim.object, "catalog.public.orders");
        assert_eq!(claim.kind, "table_alias");
        assert_eq!(claim.value, "orders");
        assert_eq!(
            claim.status,
            ClaimStatus::Candidate,
            "landed as a candidate"
        );
        // The opaque identity never appears in the emitted event stream.
        let identity_str = identity.as_str();
        let stream_json = serde_json::to_string(captured.as_slice()).unwrap_or_default();
        assert!(
            !stream_json.contains(identity_str),
            "opaque identity leaked into the event stream: {stream_json}"
        );
        claim.claim_id.clone()
    };

    // The event's claim id is the one the store actually persisted. Done
    // outside the event-lock guard — the store query awaits.
    let obj = object(&identity, "orders");
    let stored = store
        .list_claims(&obj, &[ClaimStatus::Candidate])
        .await
        .unwrap();
    assert_eq!(stored.len(), 1, "exactly one candidate stored");
    assert_eq!(stored[0].id.as_str(), claim_id.as_str());

    let _ = fs::remove_dir_all(root);
}

/// A turn that proposes a *duplicate* (the claim already exists) emits no
/// `KnowledgeProposed` — a duplicate is not a new proposal (spec P2d §5.2). The
/// provider proposes the same alias twice; only the first persists.
#[tokio::test]
async fn a_duplicate_proposal_emits_no_knowledge_proposed() {
    let root = temp_root("p2d_dup_runtime");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;

    // Seed the claim first, so both tool calls are duplicates of it.
    let obj = object(&identity, "orders");
    store
        .propose_claim(ProposeClaim {
            object: obj.clone(),
            fingerprint: crate::commands::unobserved_fingerprint(),
            referenced_columns: Vec::new(),
            payload: ClaimPayload::table_alias("orders").unwrap(),
            origin: ClaimOrigin::UserExplicit,
            initial_status: ClaimStatus::Candidate,
            evidence: None,
        })
        .await
        .unwrap();

    /// Issues the same `contract_propose` call twice, then answers — so both
    /// calls are duplicates and neither should emit.
    struct ProposeTwiceProvider {
        calls: Mutex<usize>,
    }
    #[async_trait]
    impl ChatProvider for ProposeTwiceProvider {
        fn name(&self) -> &str {
            "propose-twice"
        }
        async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            if *calls <= 2 {
                Ok(ChatResponse {
                    message: ChatMessage {
                        role: "assistant".into(),
                        content: String::new(),
                        tool_calls: vec![ToolCall {
                            id: format!("call-{calls}"),
                            name: "contract_propose".into(),
                            arguments: serde_json::json!({
                                "table": "catalog.public.orders",
                                "kind": "alias",
                                "value": "orders",
                            }),
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

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
        },
        provider: Box::new(ProposeTwiceProvider {
            calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(auto_candidate_memory());
    run_prompt_with_inputs(
        &runtime,
        inputs,
        "remember the orders alias",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        Some(store.clone()),
        None,
        None,
    )
    .await
    .expect("turn completes");

    {
        let captured = events.lock().unwrap();
        let proposed_count = captured
            .iter()
            .filter(|event| matches!(event, AgentEvent::KnowledgeProposed { .. }))
            .count();
        assert_eq!(
            proposed_count, 0,
            "a duplicate emits no KnowledgeProposed: {captured:?}"
        );
    }
    // Still exactly one candidate in the store (the seed); the duplicates did
    // not add a second. Done outside the event-lock guard — the store query
    // awaits.
    let stored = store
        .list_claims(&obj, &[ClaimStatus::Candidate])
        .await
        .unwrap();
    assert_eq!(stored.len(), 1, "the duplicates stored nothing new");

    let _ = fs::remove_dir_all(root);
}

// ===========================================================================
// Test 11: runtime turn with recall = off builds ConfiguredOff receipt and
// emits KnowledgeOutcome::Off.
// ===========================================================================

#[tokio::test]
async fn runtime_turn_with_recall_off_emits_knowledge_outcome_off() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: log.clone(),
    };
    let provider = AnswerProvider {
        answer: "done",
        log: log.clone(),
    };
    let identity = identity_for("analytics");
    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
        },
        provider: Box::new(provider),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let mut memory = default_memory();
    memory.recall = saya_config::MemoryRecall::Off;
    let runtime = test_runtime(memory);
    run_prompt_with_inputs(
        &runtime,
        inputs,
        "orders by month",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let captured = events.lock().unwrap();
    let outcome = captured
        .iter()
        .find_map(|e| match e {
            AgentEvent::KnowledgeSupplied { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .expect("KnowledgeSupplied present");
    assert_eq!(outcome, KnowledgeOutcome::Off);
}

// ===========================================================================
// Test 12: runtime turn with closed privacy gate builds PrivacyGateClosed
// receipt and emits KnowledgeOutcome::Skipped.
// ===========================================================================

#[tokio::test]
async fn runtime_turn_with_closed_privacy_gate_emits_knowledge_outcome_skipped() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: log.clone(),
    };
    let provider = AnswerProvider {
        answer: "done",
        log: log.clone(),
    };
    let identity = identity_for("analytics");
    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Anthropic,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: false, // privacy gate closed for cloud providers
            temperature: 0.0,
        },
        provider: Box::new(provider),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(default_memory());
    run_prompt_with_inputs(
        &runtime,
        inputs,
        "orders by month",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let captured = events.lock().unwrap();
    let outcome = captured
        .iter()
        .find_map(|e| match e {
            AgentEvent::KnowledgeSupplied { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .expect("KnowledgeSupplied present");
    assert_eq!(outcome, KnowledgeOutcome::Skipped);
}
