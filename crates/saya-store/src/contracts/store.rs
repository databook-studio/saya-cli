use crate::contracts::keys::object_id;
use crate::contracts::records::{
    ContractObjectId, ContractStore, ProposeClaim, ProposeOutcome, StoredClaim, StoredObject,
};
use crate::contracts::{store_reads, store_writes};
use crate::{SqliteStateStore, StoreError};
use async_trait::async_trait;
use saya_types::{ClaimId, ClaimStatus, DatabaseObjectRef, ProfileIdentity, SchemaFingerprint};
use std::time::{SystemTime, UNIX_EPOCH};

#[async_trait]
impl ContractStore for SqliteStateStore {
    async fn upsert_object(
        &self,
        object: &DatabaseObjectRef,
        fingerprint: &SchemaFingerprint,
    ) -> Result<ContractObjectId, StoreError> {
        let mut tx = self
            .pool()
            .await?
            .begin()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let id = upsert_object_in_tx(&mut tx, object, fingerprint, now()).await?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        self.secure_files()?;
        Ok(id)
    }
    async fn propose_claim(&self, request: ProposeClaim) -> Result<ProposeOutcome, StoreError> {
        store_writes::propose_claim(self, request).await
    }
    async fn get_claim(&self, id: &ClaimId) -> Result<Option<StoredClaim>, StoreError> {
        store_reads::get_claim(self, id).await
    }
    async fn list_claims(
        &self,
        object: &DatabaseObjectRef,
        statuses: &[ClaimStatus],
    ) -> Result<Vec<StoredClaim>, StoreError> {
        store_reads::list_claims(self, object, statuses).await
    }
    async fn list_objects(
        &self,
        profile: &ProfileIdentity,
    ) -> Result<Vec<StoredObject>, StoreError> {
        store_reads::list_objects(self, profile).await
    }
}

pub(crate) async fn upsert_object_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    object: &DatabaseObjectRef,
    fingerprint: &SchemaFingerprint,
    stamp: i64,
) -> Result<ContractObjectId, StoreError> {
    let id = object_id(object);
    sqlx::query("INSERT INTO contract_objects(id, profile_id, catalog_name, schema_name, object_name, object_kind, schema_fingerprint, fingerprint_version, first_seen_unix_ms, last_seen_unix_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(profile_id, catalog_name, schema_name, object_name, object_kind) DO UPDATE SET schema_fingerprint=excluded.schema_fingerprint, fingerprint_version=excluded.fingerprint_version, last_seen_unix_ms=excluded.last_seen_unix_ms")
        .bind(id.as_str())
        .bind(object.profile().as_str())
        .bind(object.catalog())
        .bind(object.schema())
        .bind(object.object())
        .bind(object.kind().as_str())
        .bind(fingerprint.as_str())
        .bind(fingerprint.version() as i64)
        .bind(stamp)
        .bind(stamp)
        .execute(&mut **tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(id)
}

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}
