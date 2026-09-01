//! Read-by-profile and read-by-object for the knowledge-items repository.
//!
//! Both reads are one query: object identity is inlined on the row, so "what
//! does SAYA know about this profile" and "what does it know about this object"
//! are each a single `SELECT` with no join. Profile scoping is absolute — the
//! profile id is bound in every query, so a read for one profile can never
//! return another's rows.

use crate::knowledge_items::KnowledgeStoreError;
use crate::knowledge_items::records::KnowledgeItem;
use crate::{SqliteStateStore, StoreError};
use saya_types::{
    ClaimOrigin, ClaimPayload, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
    KnowledgeState, ProfileIdentity,
};

/// One row's columns, in select order. `value_json` and `schema_binding_json`
/// come back as the serialised strings the store wrote; `value` is decoded
/// through `ClaimPayload`'s serde, the same form it was written under.
type ItemRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
);

const SELECT: &str = "SELECT id, profile_id, catalog, schema, object, object_kind, slot, cardinality, value_json, source, state, schema_binding_json, fingerprint_version, created_unix_ms, updated_unix_ms FROM knowledge_items";

fn decode_item(row: ItemRow) -> Result<KnowledgeItem, KnowledgeStoreError> {
    let (
        id,
        profile_id,
        catalog,
        schema,
        object,
        object_kind,
        slot,
        cardinality,
        value_json,
        source,
        state,
        schema_binding_json,
        fingerprint_version,
        created_unix_ms,
        updated_unix_ms,
    ) = row;
    let profile = ProfileIdentity::parse(&profile_id).map_err(|_| StoreError::Invalid)?;
    let kind = DatabaseObjectKind::parse(&object_kind).ok_or(StoreError::Invalid)?;
    let object_ref = DatabaseObjectRef::new(profile, &catalog, &schema, &object, kind)
        .map_err(|_| StoreError::Invalid)?;
    // A stored slot that no longer parses is a row written by a build this one
    // cannot read — fail closed as `MalformedSlot` rather than guessing.
    let slot = KnowledgeSlot::parse(&slot).ok_or(KnowledgeStoreError::MalformedSlot)?;
    let value =
        serde_json::from_str::<ClaimPayload>(&value_json).map_err(|_| StoreError::Invalid)?;
    let source = ClaimOrigin::parse(&source).ok_or(StoreError::Invalid)?;
    let state = KnowledgeState::parse(&state).ok_or(StoreError::Invalid)?;
    Ok(KnowledgeItem {
        id,
        object: object_ref,
        slot,
        cardinality_single: cardinality == "single",
        value,
        source,
        state,
        schema_binding_json,
        fingerprint_version: fingerprint_version as u32,
        created_unix_ms,
        updated_unix_ms,
    })
}

/// Every knowledge item for `profile` in one query.
pub(crate) async fn read_for_profile(
    store: &SqliteStateStore,
    profile: &ProfileIdentity,
) -> Result<Vec<KnowledgeItem>, KnowledgeStoreError> {
    let sql = format!(
        "{SELECT} WHERE profile_id=? ORDER BY catalog ASC, schema ASC, object ASC, slot ASC, id ASC"
    );
    let rows = sqlx::query_as::<_, ItemRow>(&sql)
        .bind(profile.as_str())
        .fetch_all(store.pool().await.map_err(|_| StoreError::Unavailable)?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    rows.into_iter().map(decode_item).collect()
}

/// Every knowledge item for one object in one query.
pub(crate) async fn read_for_object(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
) -> Result<Vec<KnowledgeItem>, KnowledgeStoreError> {
    let sql = format!(
        "{SELECT} WHERE profile_id=? AND catalog=? AND schema=? AND object=? AND object_kind=? ORDER BY slot ASC, id ASC"
    );
    let rows = sqlx::query_as::<_, ItemRow>(&sql)
        .bind(object.profile().as_str())
        .bind(object.catalog())
        .bind(object.schema())
        .bind(object.object())
        .bind(object.kind().as_str())
        .fetch_all(store.pool().await.map_err(|_| StoreError::Unavailable)?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    rows.into_iter().map(decode_item).collect()
}

/// Retrieve a single knowledge item by its unique ID.
pub(crate) async fn read_by_id(
    store: &SqliteStateStore,
    id: &str,
) -> Result<Option<KnowledgeItem>, KnowledgeStoreError> {
    let sql = format!("{SELECT} WHERE id=?");
    let row = sqlx::query_as::<_, ItemRow>(&sql)
        .bind(id)
        .fetch_optional(store.pool().await.map_err(|_| StoreError::Unavailable)?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    row.map(decode_item).transpose()
}

/// Retrieve all distinct database objects that have knowledge items for `profile`.
pub(crate) async fn read_objects_for_profile(
    store: &SqliteStateStore,
    profile: &ProfileIdentity,
) -> Result<Vec<DatabaseObjectRef>, KnowledgeStoreError> {
    let sql = "SELECT DISTINCT catalog, schema, object, object_kind FROM knowledge_items WHERE profile_id=? ORDER BY catalog ASC, schema ASC, object ASC";
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(sql)
        .bind(profile.as_str())
        .fetch_all(store.pool().await.map_err(|_| StoreError::Unavailable)?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let mut objects = Vec::with_capacity(rows.len());
    for (catalog, schema, object, object_kind) in rows {
        let kind = DatabaseObjectKind::parse(&object_kind).ok_or(StoreError::Invalid)?;
        let object_ref = DatabaseObjectRef::new(profile.clone(), &catalog, &schema, &object, kind)
            .map_err(|_| StoreError::Invalid)?;
        objects.push(object_ref);
    }
    Ok(objects)
}
