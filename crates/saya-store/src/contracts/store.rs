use crate::contracts::events::{ContractEvent, ForgetReason};
use crate::contracts::keys::object_id;
use crate::contracts::records::{
    ContractObjectId, ContractStore, ProposeClaim, ProposeOutcome, StoredClaim, StoredObject,
};
use crate::contracts::{store_reads, store_revise, store_transitions, store_writes};
use crate::{SqliteStateStore, StoreError};
use async_trait::async_trait;
use saya_types::{
    ClaimId, ClaimPayload, ClaimStatus, DatabaseObjectRef, ProfileIdentity, SchemaFingerprint,
};
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
    async fn confirm_claim(&self, id: &ClaimId) -> Result<StoredClaim, StoreError> {
        store_transitions::confirm_claim(self, id).await
    }
    async fn edit_claim(
        &self,
        id: &ClaimId,
        payload: ClaimPayload,
    ) -> Result<StoredClaim, StoreError> {
        store_revise::edit_claim(self, id, payload).await
    }
    async fn reject_claim(&self, id: &ClaimId) -> Result<StoredClaim, StoreError> {
        store_transitions::reject_claim(self, id).await
    }
    async fn forget_claim(&self, id: &ClaimId, reason: ForgetReason) -> Result<(), StoreError> {
        store_revise::forget_claim(self, id, reason).await
    }
    async fn mark_stale(&self, id: &ClaimId) -> Result<StoredClaim, StoreError> {
        store_transitions::mark_stale(self, id).await
    }
    async fn claim_events(
        &self,
        id: &ClaimId,
        limit: usize,
    ) -> Result<Vec<ContractEvent>, StoreError> {
        store_reads::claim_events(self, id, limit).await
    }
    async fn evidence_count(&self, id: &ClaimId) -> Result<usize, StoreError> {
        store_reads::evidence_count(self, id).await
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
