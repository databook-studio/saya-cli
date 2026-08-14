//! Transitions that rewrite a claim's stored content.
//!
//! Split from `store_transitions.rs`, which holds the status-only flips. These two
//! are different in kind: they re-run the payload admission checks and rewrite
//! `payload_json`, so they carry the size, redaction, and deduplication rules that
//! a plain status change does not.

use crate::contracts::admission;
use crate::contracts::events::ForgetReason;
use crate::contracts::keys::deduplication_key;
use crate::contracts::records::{MAX_CLAIM_PAYLOAD_BYTES, StoredClaim};
use crate::contracts::store::now;
use crate::contracts::store_reads;
use crate::contracts::store_transitions::meta_of;
use crate::{SqliteStateStore, StoreError, redact};
use saya_types::{ClaimId, ClaimStatus};

pub(crate) async fn edit_claim(
    store: &SqliteStateStore,
    id: &ClaimId,
    payload: saya_types::ClaimPayload,
) -> Result<StoredClaim, StoreError> {
    let serialized = serde_json::to_string(&payload).map_err(|_| StoreError::Invalid)?;
    if serialized.len() > MAX_CLAIM_PAYLOAD_BYTES {
        return Err(StoreError::LimitExceeded);
    }
    if redact(&serialized) != serialized {
        return Err(StoreError::Invalid);
    }
    admission::check(&serialized)?;
    let key = deduplication_key(&payload, &serialized);
    // Edit has no live table, so the edited claim records name-only snapshots
    // (empty `data_type`) — the same unknown treatment a no-schema proposal
    // gets. Fabricating a type here would be the silent coercion the spec warns
    // against. The behaviour is unchanged: pre-5a edit stored bare names too.
    let referenced = payload.referenced_column_name_snapshots();
    let referenced = serde_json::to_string(&referenced).map_err(|_| StoreError::Invalid)?;
    if redact(&referenced) != referenced {
        return Err(StoreError::Invalid);
    }
    admission::check(&referenced)?;
    let stamp = now();
    let mut tx = store
        .pool()
        .await?
        .begin()
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let (status, object_id, origin) = meta_of(&mut tx, id).await?;
    let status = ClaimStatus::parse(&status).ok_or(StoreError::Invalid)?;
    let legal = matches!(status, ClaimStatus::Candidate | ClaimStatus::Confirmed);
    if !legal {
        return Err(StoreError::Conflict);
    }
    let collision: Option<String> = sqlx::query_scalar(
        "SELECT id FROM contract_claims WHERE object_id=? AND deduplication_key=? AND id!=?",
    )
    .bind(&object_id)
    .bind(key.as_str())
    .bind(id.as_str())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if collision.is_some() {
        return Err(StoreError::Conflict);
    }
    sqlx::query("UPDATE contract_claims SET status='confirmed', payload_json=?, referenced_columns_json=?, deduplication_key=?, updated_unix_ms=? WHERE id=?")
        .bind(&serialized).bind(&referenced).bind(key.as_str()).bind(stamp).bind(id.as_str())
        .execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("INSERT INTO contract_events(claim_id, object_id, event, from_status, to_status, origin, created_unix_ms) VALUES (?, ?, 'edited', ?, 'confirmed', ?, ?)")
        .bind(id.as_str()).bind(&object_id).bind(status.as_str()).bind(&origin).bind(stamp)
        .execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    store_reads::get_claim(store, id)
        .await?
        .ok_or(StoreError::NotFound)
}

pub(crate) async fn forget_claim(
    store: &SqliteStateStore,
    id: &ClaimId,
    reason: ForgetReason,
) -> Result<(), StoreError> {
    let stamp = now();
    let mut tx = store
        .pool()
        .await?
        .begin()
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let (status, object_id, origin) = meta_of(&mut tx, id).await?;
    let status = ClaimStatus::parse(&status).ok_or(StoreError::Invalid)?;
    if status == ClaimStatus::Forgotten {
        return Err(StoreError::Conflict);
    }
    // Tombstone — per ADR 0002 section 2. The dedup key survives so a re-proposal
    // returns Duplicate { status: Forgotten } instead of silently resurrecting.
    sqlx::query("UPDATE contract_claims SET status='forgotten', payload_json='null', referenced_columns_json='[]', updated_unix_ms=? WHERE id=?")
        .bind(stamp).bind(id.as_str())
        .execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("DELETE FROM contract_evidence WHERE claim_id=?")
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("INSERT INTO contract_events(claim_id, object_id, event, from_status, to_status, origin, created_unix_ms, reason) VALUES (?, ?, 'forgotten', ?, 'forgotten', ?, ?, ?)")
        .bind(id.as_str()).bind(&object_id).bind(status.as_str()).bind(&origin).bind(stamp)
        .bind(reason.as_str())
        .execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(())
}
