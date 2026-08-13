use crate::contracts::events::{ContractEvent, ContractEventKind, ForgetReason};
use crate::contracts::keys::object_id;
use crate::contracts::records::{ContractObjectId, MAX_LISTED_OBJECTS, StoredClaim, StoredObject};
use crate::contracts::store_decode::{ClaimRow, ObjectRow, decode_claim, decode_object};
use crate::{SqliteStateStore, StoreError};
use saya_types::{ClaimId, ClaimOrigin, ClaimStatus, DatabaseObjectRef, ProfileIdentity};

type EventRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    i64,
    Option<String>,
);

pub(crate) async fn get_claim(
    store: &SqliteStateStore,
    id: &ClaimId,
) -> Result<Option<StoredClaim>, StoreError> {
    let row = sqlx::query_as::<_, ClaimRow>("SELECT c.id, c.payload_json, c.origin, c.status, c.schema_fingerprint, c.referenced_columns_json, c.created_unix_ms, c.updated_unix_ms, c.last_verified_unix_ms, o.profile_id, o.catalog_name, o.schema_name, o.object_name, o.object_kind, o.fingerprint_version FROM contract_claims c JOIN contract_objects o ON o.id = c.object_id WHERE c.id = ?")
        .bind(id.as_str())
        .fetch_optional(store.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    row.map(decode_claim).transpose()
}

pub(crate) async fn list_claims(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    statuses: &[ClaimStatus],
) -> Result<Vec<StoredClaim>, StoreError> {
    let object_id = object_id(object);
    let mut sql = String::from(
        "SELECT c.id, c.payload_json, c.origin, c.status, c.schema_fingerprint, c.referenced_columns_json, c.created_unix_ms, c.updated_unix_ms, c.last_verified_unix_ms, o.profile_id, o.catalog_name, o.schema_name, o.object_name, o.object_kind, o.fingerprint_version FROM contract_claims c JOIN contract_objects o ON o.id = c.object_id WHERE c.object_id=?",
    );
    if !statuses.is_empty() {
        sql.push_str(" AND c.status IN (");
        sql.push_str(&vec!["?"; statuses.len()].join(","));
        sql.push(')');
    }
    sql.push_str(" ORDER BY c.created_unix_ms ASC, c.id ASC");
    let mut query = sqlx::query_as::<_, ClaimRow>(&sql).bind(object_id.as_str());
    for status in statuses {
        query = query.bind(status.as_str());
    }
    let rows = query
        .fetch_all(store.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    rows.into_iter().map(decode_claim).collect()
}

pub(crate) async fn list_objects(
    store: &SqliteStateStore,
    profile: &ProfileIdentity,
) -> Result<Vec<StoredObject>, StoreError> {
    let rows = sqlx::query_as::<_, ObjectRow>("SELECT id, profile_id, catalog_name, schema_name, object_name, object_kind, schema_fingerprint, fingerprint_version, first_seen_unix_ms, last_seen_unix_ms FROM contract_objects WHERE profile_id=? ORDER BY last_seen_unix_ms DESC, id ASC LIMIT ?")
        .bind(profile.as_str())
        .bind(MAX_LISTED_OBJECTS as i64)
        .fetch_all(store.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    rows.into_iter().map(decode_object).collect()
}

pub(crate) async fn claim_events(
    store: &SqliteStateStore,
    id: &ClaimId,
    limit: usize,
) -> Result<Vec<ContractEvent>, StoreError> {
    let pool = store.pool().await?;
    let limit = limit.min(1000) as i64;

    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM contract_claims WHERE id=?)")
            .bind(id.as_str())
            .fetch_one(pool)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    if !exists {
        return Err(StoreError::NotFound);
    }

    let rows = sqlx::query_as::<_, EventRow>(
        "SELECT claim_id, object_id, event, from_status, to_status, origin, created_unix_ms, reason FROM contract_events WHERE claim_id=? ORDER BY created_unix_ms ASC, id ASC LIMIT ?",
    )
    .bind(id.as_str())
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|_| StoreError::Unavailable)?;

    rows.into_iter()
        .map(|row| {
            let (
                claim_id,
                object_id,
                event,
                from_status,
                to_status,
                origin,
                created_unix_ms,
                reason,
            ) = row;
            Ok(ContractEvent {
                claim_id: ClaimId::parse(&claim_id).map_err(|_| StoreError::Invalid)?,
                object_id: ContractObjectId::from_inner(object_id),
                kind: ContractEventKind::parse(&event).ok_or(StoreError::Invalid)?,
                from_status: from_status.as_deref().and_then(ClaimStatus::parse),
                to_status: to_status.as_deref().and_then(ClaimStatus::parse),
                reason: reason.as_deref().and_then(ForgetReason::parse),
                origin: ClaimOrigin::parse(&origin).ok_or(StoreError::Invalid)?,
                created_unix_ms,
            })
        })
        .collect()
}
