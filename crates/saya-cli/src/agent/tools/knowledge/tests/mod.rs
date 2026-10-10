//! Tests for the read-only agent contract tools (slice 2b-3a).
//!
//! These exercise the `contract_search` and `contract_read` tools end-to-end
//! through the `ToolExecutor` surface the agent uses, against a real
//! `SqliteStateStore`. They assert the privacy gate, the store-failure
//! degradation, the absence of the opaque identity, and the `ToolDefinition`
//! invariants.

use super::*;
use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutor};
use saya_connectors::DatabaseConnector;
use saya_store::{SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimOrigin, ClaimPayload, Column, ConnectionError, Database, DatabaseObjectKind,
    DatabaseObjectRef, DatabaseProfile, KnowledgeSlot, KnowledgeState, ProfileIdentity,
    QueryRequest, QueryResult, Schema, SchemaFingerprint, SchemaTree, SqlDialect, Table,
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

/// A one-column live table for `object`, so a claim stored under its
/// fingerprint reads `current` (the fingerprint matches and `table_alias` /
/// `table_description` claims reference no columns to drift). This keeps the
/// fixture honest: these tests exercise confirmed-vs-candidate filtering,
/// truncation and identity-leak — not staleness — so the claim must not be
/// stale. Storing under the all-zero `unobserved_fingerprint` against an empty
/// cached schema would compute `Stale`, which the model-facing policy now drops.
fn current_table(object: &DatabaseObjectRef) -> Table {
    Table {
        name: object.object().to_string(),
        columns: vec![Column {
            name: "id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }],
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

/// Seeds a cached schema for `object`'s profile carrying `object`'s table, and
/// returns that table's fingerprint — so a claim stored under it reads
/// `current` against the cache. Called by the remember-helpers below.
async fn seed_current(store: &SqliteStateStore, object: &DatabaseObjectRef) -> SchemaFingerprint {
    let table = current_table(object);
    let tree = SchemaTree {
        databases: vec![Database {
            name: object.catalog().to_string(),
            schemas: vec![Schema {
                name: object.schema().to_string(),
                tables: vec![table.clone()],
            }],
        }],
    };
    store
        .upsert_schema(object.profile().as_str(), &tree)
        .await
        .unwrap();
    SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table)
}

/// Seeds a knowledge item into the D-3 `knowledge_items` table both contract
/// tools read: `contract_search` via `recall`, `contract_read` via `show`
/// (Chunk 3 migrated `show` onto `knowledge_items`). The slot and
/// `SchemaBinding` are derived from the payload the way the ingest path derives
/// them. The cached schema (`seed_current`) names the object's table with an
/// `id` column so a `Table` binding reads `current`; a `Column` binding needs
/// its column present too (a test that needs a richer schema upserts its own
/// after seeding).
async fn put_knowledge_item(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    payload: ClaimPayload,
    state: KnowledgeState,
) {
    use saya_store::{KnowledgeItemRequest, KnowledgeItemStore};
    use saya_types::SchemaBinding;
    let _ = seed_current(store, object).await;
    let slot = slot_for(&payload);
    let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
    let request = KnowledgeItemRequest {
        object: object.clone(),
        slot,
        value: payload,
        source: if state == KnowledgeState::Active {
            ClaimOrigin::UserExplicit
        } else {
            ClaimOrigin::AssistantInferred
        },
        state,
        schema_binding_json: serde_json::to_string(&binding).unwrap(),
        fingerprint: crate::commands::unobserved_fingerprint(),
    };
    store.put_knowledge_item(request).await.unwrap();
}

fn slot_for(payload: &ClaimPayload) -> KnowledgeSlot {
    match payload {
        ClaimPayload::TableDescription { .. } => KnowledgeSlot::TableDescription,
        ClaimPayload::TableAlias { .. } => KnowledgeSlot::TableAlias,
        ClaimPayload::TableGrain { .. } => KnowledgeSlot::TableGrain,
        ClaimPayload::DefaultTimeColumn { .. } => KnowledgeSlot::TableDefaultTime,
        ClaimPayload::ColumnDescription { column, .. } => KnowledgeSlot::ColumnDescription {
            column: column.clone(),
        },
        ClaimPayload::ColumnRole { column, .. } => KnowledgeSlot::ColumnRole {
            column: column.clone(),
        },
        ClaimPayload::JoinRule { .. } => KnowledgeSlot::RelationJoinRule,
        ClaimPayload::MetricDefinition { .. } => KnowledgeSlot::MetricDefinition,
        _ => panic!("no slot for payload {:?}", payload),
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
// ---------------------------------------------------------------------------
// Test 2: contract_read returns one object's contract; malformed table errors.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Test 3: unknown property, non-string connection, 17-entry terms.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Test 4: no opaque identity in any tool result.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Test 5: privacy gate closed -> both tools return empty, no claims.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Test 6: unopenable store -> empty result, never Err.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Test 7: ToolDefinition invariants — read-only, no approval, not writable.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Test 7b: availability gate — tools are hidden without a store or when the
// privacy gate is closed (spec §3: hide, do not advertise-empty).
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Test 8: contract_read flags truncation past the per-object claim bound.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Test 9: contract_read on an object with no contract returns empty + reason.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Test 10: a confirmed claim computed `stale` is dropped from contract_search,
// and the result says so rather than silently returning an empty list.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Test 11: contract_read on a stale object reports the object as stale and
// returns no claims to act on.
// ---------------------------------------------------------------------------

#[path = "bounds.rs"]
mod bounds;
#[path = "definitions.rs"]
mod definitions;
#[path = "privacy.rs"]
mod privacy;
#[path = "read.rs"]
mod read;
#[path = "search.rs"]
mod search;
