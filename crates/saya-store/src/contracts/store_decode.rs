use crate::StoreError;
use crate::contracts::records::{ContractObjectId, StoredClaim, StoredObject};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, DatabaseObjectKind, DatabaseObjectRef,
    ProfileIdentity, ReferencedColumn, SchemaFingerprint,
};

pub(crate) type ObjectRow = (
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
pub(crate) type ClaimRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    Option<i64>,
    String,
    String,
    String,
    String,
    String,
    i64,
);

pub(crate) fn decode_claim(row: ClaimRow) -> Result<StoredClaim, StoreError> {
    let (
        id,
        payload_json,
        origin,
        status,
        schema_fingerprint,
        referenced_columns_json,
        created_unix_ms,
        updated_unix_ms,
        last_verified_unix_ms,
        profile_id,
        catalog_name,
        schema_name,
        object_name,
        object_kind,
        fingerprint_version,
    ) = row;
    let object = parse_ref(
        &profile_id,
        &catalog_name,
        &schema_name,
        &object_name,
        &object_kind,
    )?;
    let schema_fingerprint =
        SchemaFingerprint::from_parts(fingerprint_version as u32, &schema_fingerprint)
            .map_err(|_| StoreError::Invalid)?;
    let payload = if payload_json == "null" {
        None
    } else {
        Some(serde_json::from_str::<ClaimPayload>(&payload_json).map_err(|_| StoreError::Invalid)?)
    };
    Ok(StoredClaim {
        id: ClaimId::parse(&id).map_err(|_| StoreError::Invalid)?,
        object,
        payload,
        origin: ClaimOrigin::parse(&origin).ok_or(StoreError::Invalid)?,
        status: ClaimStatus::parse(&status).ok_or(StoreError::Invalid)?,
        schema_fingerprint,
        referenced_columns: decode_referenced_columns(&referenced_columns_json)?,
        created_unix_ms,
        updated_unix_ms,
        last_verified_unix_ms,
    })
}

pub(crate) fn decode_object(row: ObjectRow) -> Result<StoredObject, StoreError> {
    let (
        id,
        profile_id,
        catalog_name,
        schema_name,
        object_name,
        object_kind,
        schema_fingerprint,
        fingerprint_version,
        first_seen_unix_ms,
        last_seen_unix_ms,
    ) = row;
    let object = parse_ref(
        &profile_id,
        &catalog_name,
        &schema_name,
        &object_name,
        &object_kind,
    )?;
    let fingerprint =
        SchemaFingerprint::from_parts(fingerprint_version as u32, &schema_fingerprint)
            .map_err(|_| StoreError::Invalid)?;
    Ok(StoredObject {
        id: ContractObjectId::from_inner(id),
        object,
        fingerprint,
        fingerprint_version: fingerprint_version as u32,
        first_seen_unix_ms,
        last_seen_unix_ms,
    })
}

fn parse_ref(
    profile_id: &str,
    catalog: &str,
    schema: &str,
    object: &str,
    kind: &str,
) -> Result<DatabaseObjectRef, StoreError> {
    let profile = ProfileIdentity::parse(profile_id).map_err(|_| StoreError::Invalid)?;
    let kind = DatabaseObjectKind::parse(kind).ok_or(StoreError::Invalid)?;
    DatabaseObjectRef::new(profile, catalog, schema, object, kind).map_err(|_| StoreError::Invalid)
}

/// Decodes `referenced_columns_json` in either shape it was written under.
///
/// Phase 5a rows hold `[{"name","data_type","nullable"}]` snapshots. Pre-5a
/// rows — and an existing developer state database, even though the migration
/// is unreleased — hold a bare `["a","b"]` name list. A name decodes to a
/// name-only snapshot: empty `data_type`, `nullable: false`. The reconciler
/// treats an empty type as *unknown* rather than as a match, so an old row
/// never silently claims a real type. A coercion that looked like a real
/// snapshot would be worse than a missing one.
fn decode_referenced_columns(json: &str) -> Result<Vec<ReferencedColumn>, StoreError> {
    if let Ok(snapshots) = serde_json::from_str::<Vec<ReferencedColumn>>(json) {
        return Ok(snapshots);
    }
    let names = serde_json::from_str::<Vec<String>>(json).map_err(|_| StoreError::Invalid)?;
    Ok(names
        .into_iter()
        .map(|name| ReferencedColumn {
            name,
            data_type: String::new(),
            nullable: false,
        })
        .collect())
}
