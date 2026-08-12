use crate::contracts::keys::object_id;
use crate::contracts::records::{MAX_LISTED_OBJECTS, StoredClaim, StoredObject};
use crate::contracts::store_decode::{ClaimRow, ObjectRow, decode_claim, decode_object};
use crate::{SqliteStateStore, StoreError};
use saya_types::{ClaimId, ClaimStatus, DatabaseObjectRef, ProfileIdentity};

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
