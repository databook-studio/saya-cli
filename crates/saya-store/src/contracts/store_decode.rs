use crate::StoreError;
use crate::contracts::records::{ContractObjectId, StoredClaim, StoredObject};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, DatabaseObjectKind, DatabaseObjectRef,
    ProfileIdentity, SchemaFingerprint,
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
        referenced_columns: serde_json::from_str(&referenced_columns_json)
            .map_err(|_| StoreError::Invalid)?,
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
