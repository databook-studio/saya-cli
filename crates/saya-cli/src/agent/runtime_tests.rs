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
    KnowledgeOutcome, LearningSkipReason, OverrideFindingDto, ProposedClaimDto, ProviderError,
    SuppliedClaimDto, SuppliedContractDto, ToolCall,
};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig, ResolvedMemory,
};
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, Column, ConnectionError, Database,
    DatabaseObjectKind, DatabaseObjectRef, DatabaseProfile, KnowledgeSlot, KnowledgeState,
    ProfileIdentity, QueryRequest, QueryResult, Schema, SchemaTree, SqlDialect, Table,
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
        Ok(SchemaTree {
            databases: vec![Database {
                name: "catalog".into(),
                schemas: vec![Schema {
                    name: "public".into(),
                    tables: vec![orders_table()],
                }],
            }],
        })
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
    _fingerprint: &saya_types::SchemaFingerprint,
    column: &str,
    status: ClaimStatus,
    origin: ClaimOrigin,
) -> ClaimId {
    use saya_types::{KnowledgeSlot, SchemaBinding};
    let payload = ClaimPayload::default_time_column(column, None).unwrap();
    let state = match status {
        ClaimStatus::Confirmed => KnowledgeState::Active,
        ClaimStatus::Candidate => KnowledgeState::Pending,
        _ => KnowledgeState::Dismissed,
    };
    let slot = KnowledgeSlot::TableDefaultTime;
    let binding = SchemaBinding::derive(&slot, &payload).expect("default_time slot/payload agree");
    let request = KnowledgeItemRequest {
        object: obj.clone(),
        slot,
        value: payload,
        source: origin,
        state,
        schema_binding_json: serde_json::to_string(&binding).unwrap(),
        fingerprint: crate::commands::unobserved_fingerprint(),
    };
    store.put_knowledge_item(request).await.unwrap();
    // Read the store-assigned `ki-` id back so the event-naming assertion
    // compares against exactly what recall supplied.
    ClaimId::parse(
        &store
            .knowledge_for_object(obj)
            .await
            .expect("knowledge items listed")
            .into_iter()
            .find(|i| i.slot == KnowledgeSlot::TableDefaultTime)
            .expect("default_time item stored")
            .id,
    )
    .expect("ki id")
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
                timeout_seconds: 60,
                idle_timeout_seconds: 90,
                max_output_tokens: 4096,
            },
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            query_timeout_seconds: 5,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Auto,
            memory,
            ignored_project_overrides: Vec::new(),
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
        mode: MemoryMode::Off,
        max_contracts: 5,
        max_claims_per_contract: 12,
        max_context_bytes: 16384,
    }
}

/// `assisted` memory: permits candidate writes and recalls active + candidate knowledge.
fn assisted_memory() -> ResolvedMemory {
    ResolvedMemory {
        mode: MemoryMode::Assisted,
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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(provider),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(assisted_memory());
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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(provider),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(assisted_memory());
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
// Chunk 4: Post-turn structured extraction runtime integration tests
// ===========================================================================

struct TurnAndExtractionProvider {
    turn_step: Mutex<usize>,
    turn_steps: Vec<ChatResponse>,
    extraction_response: Result<ChatResponse, ProviderError>,
    extraction_calls: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for TurnAndExtractionProvider {
    fn name(&self) -> &str {
        "turn-and-extraction-provider"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        // The extraction request is the one whose system prompt identifies SAYA's
        // post-turn extractor (`build_extraction_prompt` opens with that line). A
        // turn request's system prompt is the connection context, which never
        // contains this phrase, so the two are distinguished by content — the one
        // stable marker the production prompt guarantees.
        let is_extraction = request
            .messages
            .first()
            .map(|m| m.content.contains("precision schema knowledge extractor"))
            .unwrap_or(false);
        if is_extraction {
            let mut calls = self.extraction_calls.lock().unwrap();
            *calls += 1;
            self.extraction_response.clone()
        } else {
            let mut step = self.turn_step.lock().unwrap();
            let idx = *step;
            *step += 1;
            if idx < self.turn_steps.len() {
                Ok(self.turn_steps[idx].clone())
            } else {
                Ok(ChatResponse {
                    message: ChatMessage::text("assistant", "done"),
                })
            }
        }
    }
}

/// 1. Integration test: agent runs, completes answer, harness executes extraction,
/// writes to store, and sink receives KnowledgeProposed.
#[tokio::test]
async fn test_runtime_runs_post_turn_extraction_and_emits_proposed_event() {
    let root = temp_root("post_turn_extract");
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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: vec![
                ChatResponse {
                    message: ChatMessage {
                        role: "assistant".into(),
                        content: String::new(),
                        tool_calls: vec![ToolCall {
                            id: "call-1".into(),
                            name: "bounded_sql_query".into(),
                            arguments: serde_json::json!({
                                "connection": "analytics",
                                "sql": "SELECT id, status FROM catalog.public.orders",
                            }),
                        }],
                        tool_call_id: None,
                    },
                },
                ChatResponse {
                    message: ChatMessage::text(
                        "assistant",
                        "The orders table contains customer orders.",
                    ),
                },
            ],
            extraction_response: Ok(ChatResponse {
                message: ChatMessage::text(
                    "assistant",
                    r#"{"proposals": [{"object_id": "T0", "slot": "table.alias", "value": "orders", "origin": "user_explicit"}]}"#,
                ),
            }),
            extraction_calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(assisted_memory());
    let out = run_prompt_with_inputs(
        &runtime,
        inputs,
        "table orders has alias orders",
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

    assert_eq!(out.answer, "The orders table contains customer orders.");

    // The event assertions run under the sink lock; the store query below
    // awaits, so the guard is dropped before it (holding a std `Mutex` guard
    // across an await is a clippy error and a real footgun).
    {
        let captured = events.lock().unwrap();
        let proposed: Vec<&ProposedClaimDto> = captured
            .iter()
            .filter_map(|event| match event {
                AgentEvent::KnowledgeProposed { claim } => Some(claim),
                _ => None,
            })
            .collect();
        assert_eq!(proposed.len(), 1, "exactly one KnowledgeProposed emitted");
        assert_eq!(proposed[0].profile, "analytics");
        assert_eq!(proposed[0].object, "catalog.public.orders");
        assert_eq!(proposed[0].kind, "table_alias");
        assert_eq!(proposed[0].value, "orders");
        // The user explicitly asserted the alias, so per spec F Chunk 3 +
        // `ClaimOrigin::may_confirm_directly` the proposal lands `Active`, which
        // the DTO reports as `Confirmed` — a user assertion is the act of
        // confirmation, not a candidate pending it.
        assert_eq!(proposed[0].status, ClaimStatus::Confirmed);
    }

    let obj = object(&identity, "orders");
    // Phase F persists to `knowledge_items` (D-3's projection), not the legacy
    // `contract_claims` table the retired `contract_propose` wrote to — so the
    // end-to-end persistence is asserted through the knowledge-items read.
    let stored = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(stored.len(), 1, "persisted in knowledge_items");
    assert_eq!(stored[0].slot, KnowledgeSlot::TableAlias);
    assert_eq!(stored[0].state, KnowledgeState::Active);
    assert_eq!(stored[0].source, ClaimOrigin::UserExplicit);

    let _ = fs::remove_dir_all(root);
}

/// 2. Mock provider returns error during extraction; agent output is returned successfully and unaffected (Safety Property 1).
#[tokio::test]
async fn test_runtime_extraction_failure_never_fails_turn() {
    let root = temp_root("extract_fail_safe");
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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: vec![
                ChatResponse {
                    message: ChatMessage {
                        role: "assistant".into(),
                        content: String::new(),
                        tool_calls: vec![ToolCall {
                            id: "call-1".into(),
                            name: "bounded_sql_query".into(),
                            arguments: serde_json::json!({
                                "connection": "analytics",
                                "sql": "SELECT id, status FROM catalog.public.orders",
                            }),
                        }],
                        tool_call_id: None,
                    },
                },
                ChatResponse {
                    message: ChatMessage::text(
                        "assistant",
                        "The orders table was inspected successfully.",
                    ),
                },
            ],
            extraction_response: Err(ProviderError::configuration("http 500 error")),
            extraction_calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(assisted_memory());
    let out = run_prompt_with_inputs(
        &runtime,
        inputs,
        "table orders has alias orders",
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
    .expect("turn completes despite extraction failure (fail-soft)");

    assert_eq!(out.answer, "The orders table was inspected successfully.");

    let captured = events.lock().unwrap();
    let proposed_count = captured
        .iter()
        .filter(|e| matches!(e, AgentEvent::KnowledgeProposed { .. }))
        .count();
    assert_eq!(
        proposed_count, 0,
        "no proposals emitted when extraction fails"
    );

    let _ = fs::remove_dir_all(root);
}

/// 3. When memory.mode = MemoryMode::Off, zero extraction requests occur.
#[tokio::test]
async fn test_runtime_extraction_skipped_when_memory_mode_off() {
    let root = temp_root("extract_off");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let provider = Arc::new(TurnAndExtractionProvider {
        turn_step: Mutex::new(0),
        turn_steps: vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({
                            "connection": "analytics",
                            "sql": "SELECT id, status FROM catalog.public.orders",
                        }),
                    }],
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "query completed"),
            },
        ],
        extraction_response: Ok(ChatResponse {
            message: ChatMessage::text("assistant", r#"{"proposals": []}"#),
        }),
        extraction_calls: Mutex::new(0),
    });

    struct SharedProvider(Arc<TurnAndExtractionProvider>);
    #[async_trait]
    impl ChatProvider for SharedProvider {
        fn name(&self) -> &str {
            "shared"
        }
        async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
            self.0.complete(req).await
        }
    }

    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(SharedProvider(provider.clone())),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let mut mem = assisted_memory();
    mem.mode = saya_config::MemoryMode::Off;
    let runtime = test_runtime(mem);
    let out = run_prompt_with_inputs(
        &runtime,
        inputs,
        "table orders has alias orders",
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

    assert_eq!(out.answer, "query completed");
    assert_eq!(
        *provider.extraction_calls.lock().unwrap(),
        0,
        "extraction was never called"
    );

    let _ = fs::remove_dir_all(root);
}

/// 4. Asserts contract_propose is absent from DatabaseTools::definitions(...).
#[test]
fn test_contract_propose_tool_not_advertised_to_model() {
    let tools = super::tools::DatabaseTools::definitions(true, true, true);
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        !names.contains(&"contract_propose"),
        "contract_propose must not be advertised"
    );
    assert!(names.contains(&"schema_discovery"));
    assert!(names.contains(&"bounded_sql_query"));
    assert!(names.contains(&"contract_search"));
    assert!(names.contains(&"contract_read"));
}

/// 5. Supplied contract in recall receipt prevents duplicate candidate proposal from being emitted or stored during the turn (Safety Property 4).
#[tokio::test]
async fn test_anti_self_reinforcement_end_to_end() {
    let root = temp_root("anti_self_reinforce_e2e");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;

    // Seed confirmed claim on orders
    let obj = object(&identity, "orders");
    let tree = orders_schema(&identity);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&orders_table());
    let _ = remember_default_time_column(
        &store,
        &obj,
        &fp,
        "created_at",
        ClaimStatus::Confirmed,
        ClaimOrigin::UserExplicit,
    )
    .await;

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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: vec![
                ChatResponse {
                    message: ChatMessage {
                        role: "assistant".into(),
                        content: String::new(),
                        tool_calls: vec![ToolCall {
                            id: "call-1".into(),
                            name: "bounded_sql_query".into(),
                            arguments: serde_json::json!({
                                "connection": "analytics",
                                "sql": "SELECT id, created_at FROM catalog.public.orders",
                            }),
                        }],
                        tool_call_id: None,
                    },
                },
                ChatResponse {
                    message: ChatMessage::text("assistant", "Order dates checked."),
                },
            ],
            // The model re-infers the *same* default-time claim recall already
            // supplied (`default_time_column=created_at` → slot `table.default_time`,
            // value `created_at`), as `assistant_inferred`. Anti-self-reinforcement
            // drops it as an exact duplicate of the supplied claim — the property
            // this test exists for. A different-slot inference would NOT be dropped,
            // so the fixture must duplicate the supplied slot+value to exercise it.
            extraction_response: Ok(ChatResponse {
                message: ChatMessage::text(
                    "assistant",
                    r#"{"proposals": [{"object_id": "T0", "slot": "table.default_time", "value": "created_at", "origin": "assistant_inferred"}]}"#,
                ),
            }),
            extraction_calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(assisted_memory());
    let out = run_prompt_with_inputs(
        &runtime,
        inputs,
        "show me orders",
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

    assert_eq!(out.answer, "Order dates checked.");

    let captured = events.lock().unwrap();
    let proposed_count = captured
        .iter()
        .filter(|e| matches!(e, AgentEvent::KnowledgeProposed { .. }))
        .count();
    assert_eq!(
        proposed_count, 0,
        "anti-self-reinforcement dropped duplicate inference"
    );

    let _ = fs::remove_dir_all(root);
}

// ===========================================================================
// Test 11: runtime turn with mode = off builds ConfiguredOff receipt and
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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(provider),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let mut memory = default_memory();
    memory.mode = saya_config::MemoryMode::Off;
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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(provider),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(assisted_memory());
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

// ===========================================================================
// Spec A1: surfacing an override from the SQL, not the model's confession.
//
// `detect_overrides` exists and is tested against the real generated SQL; this
// slice wires it into the turn and emits `KnowledgeOverridden`. These tests
// drive a turn through `run_prompt_with_inputs` with a provider that issues one
// `bounded_sql_query` call, so detection runs against the statement the model
// actually generated and the receipt recall supplied — not a unit oracle.
//
// The harness mirrors test 1 above: a confirmed `default_time_column` claim of
// `return_date` seeded under a matching cached `orders` schema (columns
// `id` + `created_at`), so recall supplies the claim as `Current`. The claim
// names `return_date` as the time column; the model's SQL references a
// *different* time-named column (`rental_date`) on the same object — the live
// override case the detector exists to catch.
// ===========================================================================

/// `mode = Assisted` supplies confirmed claims for override detection.
fn a1_memory() -> ResolvedMemory {
    ResolvedMemory {
        mode: MemoryMode::Assisted,
        max_contracts: 5,
        max_claims_per_contract: 12,
        max_context_bytes: 16384,
    }
}

/// `mode = Assisted` supplies both confirmed and candidate claims.
fn a1_include_candidates_memory() -> ResolvedMemory {
    ResolvedMemory {
        mode: MemoryMode::Assisted,
        max_contracts: 5,
        max_claims_per_contract: 12,
        max_context_bytes: 16384,
    }
}

/// A provider that issues one `bounded_sql_query` call with `sql` on the first
/// request and a text answer on every subsequent one — a one-query turn that
/// completes, so the runtime drains the override log and emits after the loop.
struct QueryProvider {
    sql: &'static str,
    calls: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for QueryProvider {
    fn name(&self) -> &str {
        "query-once"
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
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({ "sql": self.sql }),
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

/// The live override statement: a different time-named column referenced on the
/// claimed object, the claimed column absent.
const OVERRIDE_SQL: &str = "SELECT rental_date FROM orders WHERE rental_date > '2024-01-01'";

/// Seeds a confirmed `default_time_column` claim of `return_date` on
/// `catalog.public.orders` under a matching cached schema, returning the temp
/// root, the claim id, and the open store. Mirrors the test-1 harness.
///
/// D-4 NOTE: the claim's binding is `Column { return_date, Time }`, so the
/// cached schema must carry `return_date` as a temporal column for the item to
/// read `current` (and so reach the model, where override detection sees it).
/// The old whole-table fingerprint model classified the claim `Current` from
/// the fingerprint match alone; D-4 classifies from the binding, so the schema
/// must name the bound column.
async fn a1_turn_setup(
    status: ClaimStatus,
    origin: ClaimOrigin,
) -> (PathBuf, ClaimId, SqliteStateStore) {
    let root = temp_root("a1");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let obj = object(&identity, "orders");
    let tree = SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: "orders".into(),
                    columns: vec![
                        Column {
                            name: "id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                        },
                        Column {
                            name: "return_date".into(),
                            data_type: "timestamp".into(),
                            nullable: false,
                        },
                    ],
                }],
            }],
        }],
    };
    store.upsert_schema(identity.as_str(), &tree).await.unwrap();
    let fp = live_fingerprint(&orders_table());
    let claim_id =
        remember_default_time_column(&store, &obj, &fp, "return_date", status, origin).await;
    (root, claim_id, store)
}

/// The single `KnowledgeOverridden` event in `captured`, if any. Detection
/// emits at most one event per turn carrying every finding.
fn one_overridden(captured: &[AgentEvent]) -> Option<&[OverrideFindingDto]> {
    captured.iter().find_map(|event| match event {
        AgentEvent::KnowledgeOverridden { findings } => Some(findings.as_slice()),
        _ => None,
    })
}

// Test 1: a turn whose SQL contradicts a supplied confirmed claim emits one
// `KnowledgeOverridden` naming it.
#[tokio::test]
async fn a_turn_contradicting_a_confirmed_claim_emits_one_knowledge_overridden() {
    let (root, claim_id, store) =
        a1_turn_setup(ClaimStatus::Confirmed, ClaimOrigin::UserExplicit).await;

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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(QueryProvider {
            sql: OVERRIDE_SQL,
            calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity_for("analytics")),
        failures: Vec::new(),
    };
    let runtime = test_runtime(a1_memory());
    run_prompt_with_inputs(
        &runtime,
        inputs,
        "orders by month",
        saya_agent::ApprovalPolicy::ReadOnly,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        // The store must be present so recall supplies the claim.
        Some(store),
        None,
        None,
    )
    .await
    .expect("turn completes");

    let captured = events.lock().unwrap();
    let findings = one_overridden(&captured).expect("one KnowledgeOverridden");
    assert_eq!(
        findings.len(),
        1,
        "exactly one finding, naming the contradicted claim: {captured:?}"
    );
    let f = &findings[0];
    assert_eq!(f.claim_id, claim_id, "names the supplied confirmed claim");
    assert_eq!(f.kind, "default_time_column");
    assert_eq!(f.claimed_value, "return_date", "where you specified Y");
    assert!(
        f.observed_columns.contains(&"rental_date".to_string()),
        "names the column actually referenced: {f:?}"
    );
    let _ = fs::remove_dir_all(root);
}

// Test 2: a turn whose SQL honours the claim emits nothing.
#[tokio::test]
async fn a_turn_honouring_the_claim_emits_no_knowledge_overridden() {
    let (root, _claim_id, store) =
        a1_turn_setup(ClaimStatus::Confirmed, ClaimOrigin::UserExplicit).await;
    // The claimed column `return_date` is referenced → the claim is honoured.
    let honoring_sql = "SELECT return_date FROM orders WHERE return_date > '2024-01-01'";

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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(QueryProvider {
            sql: honoring_sql,
            calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity_for("analytics")),
        failures: Vec::new(),
    };
    let runtime = test_runtime(a1_memory());
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
    .expect("turn completes");

    let captured = events.lock().unwrap();
    assert!(
        one_overridden(&captured).is_none(),
        "honouring the claim must emit no override event: {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}

// Test 3: unparseable SQL emits nothing.
#[tokio::test]
async fn a_turn_with_unparseable_sql_emits_no_knowledge_overridden() {
    let (root, _claim_id, store) =
        a1_turn_setup(ClaimStatus::Confirmed, ClaimOrigin::UserExplicit).await;
    // `sql_references` returns `None` for this; the detector fails closed.
    let unparseable_sql = "SELECT FROM WHERE";

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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(QueryProvider {
            sql: unparseable_sql,
            calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity_for("analytics")),
        failures: Vec::new(),
    };
    let runtime = test_runtime(a1_memory());
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
    .expect("turn completes");

    let captured = events.lock().unwrap();
    assert!(
        one_overridden(&captured).is_none(),
        "unparseable SQL must emit no override event: {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}

// Test 4: a candidate claim contradicted emits nothing (only confirmed claims
// can be overridden). `recall = IncludeCandidates` so the candidate IS supplied
// to the model — the detector's status guard is what suppresses the finding,
// not recall's filter.
#[tokio::test]
async fn a_candidate_claim_contradicted_emits_nothing() {
    let (root, _claim_id, store) =
        a1_turn_setup(ClaimStatus::Candidate, ClaimOrigin::UserExplicit).await;

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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(QueryProvider {
            sql: OVERRIDE_SQL,
            calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity_for("analytics")),
        failures: Vec::new(),
    };
    let runtime = test_runtime(a1_include_candidates_memory());
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
    .expect("turn completes");

    let captured = events.lock().unwrap();
    // The candidate IS supplied (IncludeCandidates), so KnowledgeSupplied is
    // present — but no KnowledgeOverridden fires: only a confirmed claim binds.
    assert!(
        captured
            .iter()
            .any(|e| matches!(e, AgentEvent::KnowledgeSupplied { .. })),
        "the candidate was supplied: {captured:?}"
    );
    assert!(
        one_overridden(&captured).is_none(),
        "a candidate claim is not overridable: {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}

// Test 6: no opaque profile identity leaks into the serialized event. The DTO
// has no identity field by construction; this asserts the event stream inherits
// that guarantee (mirrors the P2d identity-leak test).
#[tokio::test]
async fn no_identity_leaks_into_the_knowledge_overridden_event() {
    let (root, _claim_id, store) =
        a1_turn_setup(ClaimStatus::Confirmed, ClaimOrigin::UserExplicit).await;
    let identity_str = identity_for("analytics").as_str().to_string();

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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(QueryProvider {
            sql: OVERRIDE_SQL,
            calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity_for("analytics")),
        failures: Vec::new(),
    };
    let runtime = test_runtime(a1_memory());
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
    .expect("turn completes");

    let captured = events.lock().unwrap();
    let stream_json = serde_json::to_string(captured.as_slice()).unwrap_or_default();
    assert!(
        !stream_json.contains(&identity_str),
        "opaque identity leaked into the event stream: {stream_json}"
    );
    let _ = fs::remove_dir_all(root);
}

// ===========================================================================
// Spec packet-54: a turn whose post-turn extraction times out or errors must
// say so (KnowledgeLearningSkipped). Today it is silent — the red tests below
// assert the event fires AND the turn still completes. The gate-declined case
// emits nothing (decision 2).
//
// The timeout test sleeps *past* the production `EXTRACTION_TIMEOUT` constant
// (15s) — the spec mandates a documented, bounded constant and a test that
// sleeps past it, so this is one ~15s test by design, not a parameterized
// shortcut. The extraction call is distinguished from the turn call by the
// `precision schema knowledge extractor` system-prompt marker, the same stable
// marker `TurnAndExtractionProvider` relies on above.
// ===========================================================================

/// A provider that answers the turn normally but sleeps past the extraction
/// timeout when called for extraction, so the runtime's `tokio::time::timeout`
/// fires. Reuses the turn-steps + extraction-marker shape of
/// `TurnAndExtractionProvider`.
struct SleepingExtractionProvider {
    turn_step: Mutex<usize>,
    turn_steps: Vec<ChatResponse>,
    extraction_calls: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for SleepingExtractionProvider {
    fn name(&self) -> &str {
        "sleeping-extraction-provider"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let is_extraction = request
            .messages
            .first()
            .map(|m| m.content.contains("precision schema knowledge extractor"))
            .unwrap_or(false);
        if is_extraction {
            {
                let mut calls = self.extraction_calls.lock().unwrap();
                *calls += 1;
            }
            // Sleep past the production timeout so `tokio::time::timeout` fires.
            tokio::time::sleep(
                super::super::learning::EXTRACTION_TIMEOUT + std::time::Duration::from_secs(1),
            )
            .await;
            Ok(ChatResponse {
                message: ChatMessage::text("assistant", r#"{"proposals": []}"#),
            })
        } else {
            let mut step = self.turn_step.lock().unwrap();
            let idx = *step;
            *step += 1;
            if idx < self.turn_steps.len() {
                Ok(self.turn_steps[idx].clone())
            } else {
                Ok(ChatResponse {
                    message: ChatMessage::text("assistant", "done"),
                })
            }
        }
    }
}

/// One turn that issues a `bounded_sql_query` (object activity + non-trivial
/// answer) so the gate admits extraction, then the extraction call sleeps past
/// the timeout. Asserts `KnowledgeLearningSkipped { TimedOut }` is emitted and
/// the turn still completes with its answer (Safety Property 1: fail-soft).
#[tokio::test]
async fn a_turn_whose_extraction_times_out_emits_learning_skipped_and_completes() {
    let root = temp_root("p54_timeout");
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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(SleepingExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: vec![
                ChatResponse {
                    message: ChatMessage {
                        role: "assistant".into(),
                        content: String::new(),
                        tool_calls: vec![ToolCall {
                            id: "call-1".into(),
                            name: "bounded_sql_query".into(),
                            arguments: serde_json::json!({
                                "connection": "analytics",
                                "sql": "SELECT id, status FROM catalog.public.orders",
                            }),
                        }],
                        tool_call_id: None,
                    },
                },
                ChatResponse {
                    message: ChatMessage::text(
                        "assistant",
                        "The orders table contains customer orders.",
                    ),
                },
            ],
            extraction_calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(assisted_memory());
    let out = run_prompt_with_inputs(
        &runtime,
        inputs,
        "table orders has alias orders",
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
    .expect("turn completes despite extraction timeout (fail-soft)");

    // The turn's answer is unaffected — extraction failure is not answer failure.
    assert_eq!(out.answer, "The orders table contains customer orders.");

    let captured = events.lock().unwrap();
    let skipped = captured.iter().find_map(|event| match event {
        AgentEvent::KnowledgeLearningSkipped { reason } => Some(*reason),
        _ => None,
    });
    assert_eq!(
        skipped,
        Some(LearningSkipReason::TimedOut),
        "timeout must emit KnowledgeLearningSkipped{{TimedOut}}: {captured:?}"
    );
    // No proposal was emitted — the timeout aborted extraction before ingest.
    let proposed_count = captured
        .iter()
        .filter(|e| matches!(e, AgentEvent::KnowledgeProposed { .. }))
        .count();
    assert_eq!(
        proposed_count, 0,
        "no proposals after timeout: {captured:?}"
    );
    let _ = fs::remove_dir_all(root);
}

/// A gate-declined turn emits **nothing** for learning (decision 2: a gate skip
/// stays silent). Uses an `Off` memory mode so `permit_candidate_writes` is
/// false and the extraction block is never entered — the same path a gate
/// decline would take when the runtime skips it. Asserts no
/// `KnowledgeLearningSkipped` and no `KnowledgeProposed` appears.
#[tokio::test]
async fn a_gate_declined_turn_emits_no_learning_event() {
    let root = temp_root("p54_gate_decline");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let provider = Arc::new(SleepingExtractionProvider {
        turn_step: Mutex::new(0),
        // A trivial turn with no tool call and a short answer: the gate would
        // decline (no object activity, <15-char answer). Memory is Off, so the
        // extraction block is never entered regardless — proving the silent path.
        turn_steps: vec![ChatResponse {
            message: ChatMessage::text("assistant", "ok"),
        }],
        extraction_calls: Mutex::new(0),
    });
    struct SharedProvider(Arc<SleepingExtractionProvider>);
    #[async_trait]
    impl ChatProvider for SharedProvider {
        fn name(&self) -> &str {
            "shared-sleeping"
        }
        async fn complete(&self, req: ChatRequest) -> Result<ChatResponse, ProviderError> {
            self.0.complete(req).await
        }
    }
    let inputs = TurnInputs {
        ai: ResolvedAi {
            provider: AiProvider::Ollama,
            model: "test-model".into(),
            base_url: None,
            api_key: None,
            allow_data_sharing: true,
            temperature: 0.0,
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(SharedProvider(provider.clone())),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let mut mem = assisted_memory();
    mem.mode = saya_config::MemoryMode::Off;
    let runtime = test_runtime(mem);
    let out = run_prompt_with_inputs(
        &runtime,
        inputs,
        "hi",
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
    .expect("turn completes");

    assert_eq!(out.answer, "ok");
    let captured = events.lock().unwrap();
    assert!(
        !captured
            .iter()
            .any(|e| matches!(e, AgentEvent::KnowledgeLearningSkipped { .. })),
        "a gate decline must stay silent: {captured:?}"
    );
    assert!(
        !captured
            .iter()
            .any(|e| matches!(e, AgentEvent::KnowledgeProposed { .. })),
        "no proposals on a gate-declined turn: {captured:?}"
    );
    // Extraction was never called — the gate/permit guard held.
    assert_eq!(
        *provider.extraction_calls.lock().unwrap(),
        0,
        "extraction never ran"
    );
    let _ = fs::remove_dir_all(root);
}

/// A turn whose extraction *errors* (provider failure) emits
/// `KnowledgeLearningSkipped { Failed }` — the non-timeout arm — and still
/// completes. Reuses `TurnAndExtractionProvider` with an `Err` extraction
/// response, the same harness `test_runtime_extraction_failure_never_fails_turn`
/// uses, but asserts the new event (the older test predates it and only
/// asserts no proposals).
#[tokio::test]
async fn a_turn_whose_extraction_errors_emits_learning_skipped_failed_and_completes() {
    let root = temp_root("p54_failed");
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
            timeout_seconds: 60,
            idle_timeout_seconds: 90,
            max_output_tokens: 4096,
        },
        provider: Box::new(TurnAndExtractionProvider {
            turn_step: Mutex::new(0),
            turn_steps: vec![
                ChatResponse {
                    message: ChatMessage {
                        role: "assistant".into(),
                        content: String::new(),
                        tool_calls: vec![ToolCall {
                            id: "call-1".into(),
                            name: "bounded_sql_query".into(),
                            arguments: serde_json::json!({
                                "connection": "analytics",
                                "sql": "SELECT id, status FROM catalog.public.orders",
                            }),
                        }],
                        tool_call_id: None,
                    },
                },
                ChatResponse {
                    message: ChatMessage::text(
                        "assistant",
                        "The orders table was inspected successfully.",
                    ),
                },
            ],
            extraction_response: Err(ProviderError::configuration("http 500 error")),
            extraction_calls: Mutex::new(0),
        }),
        registry: registry_for("analytics", &identity),
        failures: Vec::new(),
    };
    let runtime = test_runtime(assisted_memory());
    let out = run_prompt_with_inputs(
        &runtime,
        inputs,
        "table orders has alias orders",
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
    .expect("turn completes despite extraction error (fail-soft)");

    assert_eq!(out.answer, "The orders table was inspected successfully.");

    let captured = events.lock().unwrap();
    let skipped = captured.iter().find_map(|event| match event {
        AgentEvent::KnowledgeLearningSkipped { reason } => Some(*reason),
        _ => None,
    });
    assert_eq!(
        skipped,
        Some(LearningSkipReason::Failed),
        "error must emit KnowledgeLearningSkipped{{Failed}}, not TimedOut: {captured:?}"
    );
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(root);
}
