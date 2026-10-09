//! A013 regression: a malformed pending row must not vanish silently.
//!
//! A `knowledge_items` row whose `id` is not a valid [`ClaimId`] (a row written
//! by an incompatible build) is dropped from the queue candidate list, but the
//! queue must say so: every surviving candidate carries `incomplete`, and the
//! human renderer prints the incomplete notice. The valid pending candidate
//! still queues normally beside the malformed row. `show` applies the same
//! visible-incompleteness rule to its selected rows.

use saya_cli::{
    ContractsCommand, RenderFormat, RuntimeConfig, capture_output_start, capture_output_take,
    load_with_sources, profile_identity, run_contracts,
};
use saya_store::{KnowledgeItemRequest, KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
    KnowledgeState, ProfileIdentity, SchemaBinding, SchemaFingerprint, SchemaTree,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "queue_incomplete_ids/queue.rs"]
mod queue;
#[path = "queue_incomplete_ids/show.rs"]
mod show;

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-queue-incomplete-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn runtime_at(root: &Path) -> RuntimeConfig {
    let database = root.join("data.sqlite3");
    fs::write(&database, b"").unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();
    let options = saya_cli::GlobalOptions {
        connections: Some(connections),
        ..Default::default()
    };
    load_with_sources(&options, root, root, BTreeMap::new()).unwrap()
}

fn identity_for(runtime: &RuntimeConfig, name: &str) -> String {
    let profile = runtime.named_profile(name).unwrap();
    profile_identity(name, profile, &runtime.cache_scope)
        .as_str()
        .to_owned()
}

async fn store_at(root: &Path, runtime: &RuntimeConfig) -> SqliteStateStore {
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    store
        .upsert_schema(&identity_for(runtime, "local"), &SchemaTree::default())
        .await
        .unwrap();
    store
}

fn object_for(runtime: &RuntimeConfig, table: &str) -> DatabaseObjectRef {
    let identity = identity_for(runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        table,
        DatabaseObjectKind::Table,
    )
    .unwrap()
}

fn unobserved_fingerprint() -> SchemaFingerprint {
    SchemaFingerprint::from_parts(saya_types::FINGERPRINT_VERSION, &"0".repeat(64)).unwrap()
}

async fn seed_pending_alias(store: &SqliteStateStore, object: &DatabaseObjectRef, alias: &str) {
    let payload = ClaimPayload::table_alias(alias).unwrap();
    let slot = KnowledgeSlot::TableAlias;
    let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: object.clone(),
            slot,
            value: payload,
            source: ClaimOrigin::AssistantInferred,
            state: KnowledgeState::Pending,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            fingerprint: unobserved_fingerprint(),
        })
        .await
        .unwrap();
}

/// Plants a `Pending` row whose `id` is not a valid [`ClaimId`] — the malformed
/// selected/pending-ID fixture (`!` and a space both fail `ClaimId::parse`).
/// The store write path can never create such a row (it derives `ki-…` ids),
/// so the fixture writes it directly, after closing the store so the direct
/// connection sees a quiescent database.
async fn plant_malformed_pending_id(
    _store: &SqliteStateStore,
    db: &Path,
    object: &DatabaseObjectRef,
) {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap();
    let value_json = serde_json::to_string(&ClaimPayload::table_alias("malformed-row").unwrap())
        .expect("alias serializes");
    sqlx::query(
        "INSERT INTO knowledge_items(id, profile_id, catalog, schema, object, object_kind, slot, cardinality, value_json, source, state, schema_binding_json, cleanup_state, fingerprint_version, created_unix_ms, updated_unix_ms) VALUES ('not a valid id!', ?, ?, ?, ?, ?, 'table.alias', 'multi', ?, 'assistant_inferred', 'pending', '{}', 'complete', 1, 1, 2)",
    )
    .bind(object.profile().as_str())
    .bind(object.catalog())
    .bind(object.schema())
    .bind(object.object())
    .bind(object.kind().as_str())
    .bind(value_json)
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
}

async fn run(
    command: ContractsCommand,
    runtime: &RuntimeConfig,
    store: &SqliteStateStore,
    format: RenderFormat,
) -> (i32, String, String) {
    capture_output_start();
    let code = run_contracts(command, runtime, format, store)
        .await
        .unwrap();
    let (out, err) = capture_output_take();
    (code, out, err)
}
