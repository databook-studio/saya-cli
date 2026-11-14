use saya_store::{
    KnowledgeItemRequest, KnowledgeItemStore, KnowledgeItemsQuery, KnowledgeObjectsQuery,
    KnowledgeStoreError, MAX_KNOWLEDGE_PAGE_SIZE, MAX_SCHEMA_BINDING_BYTES, SqliteStateStore,
    StoreError,
};
use saya_types::{
    ClaimOrigin, ClaimPayload, ColumnRole, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
    KnowledgeState, ProfileIdentity, SchemaFingerprint,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-knowledge-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn profile(hex_char: char) -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", hex_char.to_string().repeat(64))).unwrap()
}

fn object(profile: &ProfileIdentity, name: &str) -> DatabaseObjectRef {
    DatabaseObjectRef::new(
        profile.clone(),
        "catalog",
        "public",
        name,
        DatabaseObjectKind::Table,
    )
    .unwrap()
}

/// A binding whose version is not the current `FINGERPRINT_VERSION`, to prove
/// the version round-trips rather than being laundered through the build's
/// constant on read.
fn binding(version: u32) -> (SchemaFingerprint, String) {
    (
        SchemaFingerprint::from_parts(version, &"f".repeat(64)).unwrap(),
        r#"{"columns":["user_id"]}"#.to_owned(),
    )
}

fn request(
    object: &DatabaseObjectRef,
    slot: KnowledgeSlot,
    value: ClaimPayload,
    source: ClaimOrigin,
    state: KnowledgeState,
    version: u32,
) -> KnowledgeItemRequest {
    let (fingerprint, schema_binding_json) = binding(version);
    KnowledgeItemRequest {
        object: object.clone(),
        slot,
        value,
        source,
        state,
        schema_binding_json,
        fingerprint,
    }
}

async fn read_pool(db: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap()
}

fn request_knowledge(
    object: &DatabaseObjectRef,
    slot: KnowledgeSlot,
    value: ClaimPayload,
) -> KnowledgeItemRequest {
    request(
        object,
        slot,
        value,
        ClaimOrigin::UserExplicit,
        KnowledgeState::Active,
        1,
    )
}

#[path = "knowledge_items/fingerprint_versions.rs"]
mod fingerprint_versions;
#[path = "knowledge_items/item_lifecycle.rs"]
mod item_lifecycle;

#[path = "knowledge_items/paging.rs"]
mod paging;
#[path = "knowledge_items/profile_reads.rs"]
mod profile_reads;
#[path = "knowledge_items/slot_constraints.rs"]
mod slot_constraints;
#[path = "knowledge_items/validation.rs"]
mod validation;
#[path = "knowledge_items/writes.rs"]
mod writes;
