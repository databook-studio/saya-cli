//! The physical row contract of the `knowledge_items` table and the decode
//! from a raw row into a `KnowledgeItem`.

use crate::StoreError;
use crate::knowledge_items::KnowledgeStoreError;
use crate::knowledge_items::records::{CleanupState, KnowledgeItem};
use saya_types::{
    ClaimOrigin, ClaimPayload, DatabaseObjectKind, DatabaseObjectRef, KnowledgeSlot,
    KnowledgeState, ProfileIdentity,
};

/// One row's columns, in select order. `value_json` and `schema_binding_json`
/// come back as the serialised strings the store wrote; `value` is decoded
/// through `ClaimPayload`'s serde, the same form it was written under.
pub(super) type ItemRow = (
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
    String,
    i64,
    i64,
    i64,
);

pub(super) const SELECT: &str = "SELECT id, profile_id, catalog, schema, object, object_kind, slot, cardinality, value_json, source, state, schema_binding_json, cleanup_state, fingerprint_version, created_unix_ms, updated_unix_ms FROM knowledge_items";

pub(super) fn decode_item(row: ItemRow) -> Result<KnowledgeItem, KnowledgeStoreError> {
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
        cleanup_state,
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
    let cleanup = CleanupState::parse(&cleanup_state).ok_or(StoreError::Invalid)?;
    Ok(KnowledgeItem {
        id,
        object: object_ref,
        slot,
        cardinality_single: cardinality == "single",
        value,
        source,
        state,
        cleanup,
        schema_binding_json,
        fingerprint_version: u32::try_from(fingerprint_version).map_err(|_| StoreError::Invalid)?,
        created_unix_ms,
        updated_unix_ms,
    })
}
