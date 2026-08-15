use crate::contracts::records::StoredClaim;
use crate::contracts::store::now;
use crate::contracts::store_reads;
use crate::{SqliteStateStore, StoreError};
use saya_types::{ClaimId, ClaimStatus};

type ClaimMeta = (String, String, String);

pub(crate) async fn meta_of(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &ClaimId,
) -> Result<ClaimMeta, StoreError> {
    sqlx::query_as::<_, ClaimMeta>(
        "SELECT status, object_id, origin FROM contract_claims WHERE id=?",
    )
    .bind(id.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| StoreError::Unavailable)?
    .ok_or(StoreError::NotFound)
}

pub(crate) async fn confirm_claim(
    store: &SqliteStateStore,
    id: &ClaimId,
) -> Result<StoredClaim, StoreError> {
    let stamp = now();
    let mut tx = store
        .pool()
        .await?
        .begin()
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let (status, object_id, origin) = meta_of(&mut tx, id).await?;
    let status = ClaimStatus::parse(&status).ok_or(StoreError::Invalid)?;
    // Status-only confirmation accepts Candidate and Contradicted only. A
    // Stale claim must take the schema-aware `revalidate_claim` path: a
    // status-only flip left the stored fingerprint untouched, so the next read
    // recomputed the digest and returned Stale again — a silent no-op the user
    // could not repair. See `store_revise::revalidate_claim`.
    let legal = matches!(status, ClaimStatus::Candidate | ClaimStatus::Contradicted);
    if !legal {
        return Err(StoreError::Conflict);
    }
    sqlx::query("UPDATE contract_claims SET status='confirmed', last_verified_unix_ms=?, updated_unix_ms=? WHERE id=?")
        .bind(stamp).bind(stamp).bind(id.as_str())
        .execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("INSERT INTO contract_events(claim_id, object_id, event, from_status, to_status, origin, created_unix_ms) VALUES (?, ?, 'confirmed', ?, 'confirmed', ?, ?)")
        .bind(id.as_str()).bind(&object_id).bind(status.as_str()).bind(&origin).bind(stamp)
        .execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    store_reads::get_claim(store, id)
        .await?
        .ok_or(StoreError::NotFound)
}

pub(crate) async fn reject_claim(
    store: &SqliteStateStore,
    id: &ClaimId,
) -> Result<StoredClaim, StoreError> {
    let stamp = now();
    let mut tx = store
        .pool()
        .await?
        .begin()
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let (status, object_id, origin) = meta_of(&mut tx, id).await?;
    let status = ClaimStatus::parse(&status).ok_or(StoreError::Invalid)?;
    if status != ClaimStatus::Candidate {
        return Err(StoreError::Conflict);
    }
    sqlx::query("UPDATE contract_claims SET status='rejected', updated_unix_ms=? WHERE id=?")
        .bind(stamp)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("INSERT INTO contract_events(claim_id, object_id, event, from_status, to_status, origin, created_unix_ms) VALUES (?, ?, 'rejected', ?, 'rejected', ?, ?)")
        .bind(id.as_str()).bind(&object_id).bind(status.as_str()).bind(&origin).bind(stamp)
        .execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    store_reads::get_claim(store, id)
        .await?
        .ok_or(StoreError::NotFound)
}

pub(crate) async fn mark_stale(
    store: &SqliteStateStore,
    id: &ClaimId,
) -> Result<StoredClaim, StoreError> {
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
    sqlx::query("UPDATE contract_claims SET status='stale', updated_unix_ms=? WHERE id=?")
        .bind(stamp)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("INSERT INTO contract_events(claim_id, object_id, event, from_status, to_status, origin, created_unix_ms) VALUES (?, ?, 'marked_stale', ?, 'stale', ?, ?)")
        .bind(id.as_str()).bind(&object_id).bind(status.as_str()).bind(&origin).bind(stamp)
        .execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    store_reads::get_claim(store, id)
        .await?
        .ok_or(StoreError::NotFound)
}
