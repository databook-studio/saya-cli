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

/// Mark every claim in `ids` stale in **one transaction** — the bulk
/// counterpart to [`mark_stale`]. Reconciliation collected the claims the 5b
/// rule computed `Stale`; persisting them one transaction each was an N+1 of
/// `BEGIN`/`COMMIT` round trips, and worse, each commit was independent so a
/// store failure mid-run left earlier claims already marked — partial state.
/// Here every transition and its audit event move in one transaction, so a
/// store error rolls the whole batch back and leaves no partial state.
///
/// A claim whose status changed between the reconcile snapshot and this call
/// (a race that flipped it out of `Candidate`/`Confirmed`) is skipped — its
/// transition is not legal, so it is not marked and not audited, exactly as the
/// per-claim `mark_stale` would have refused with `Conflict`. Such a claim is
/// still counted as *examined* by the caller; only `marked_stale` omits it.
/// Returns the number of claims actually transitioned. An empty input is a
/// no-op that opens no transaction.
pub(crate) async fn mark_stale_batch(
    store: &SqliteStateStore,
    ids: &[ClaimId],
) -> Result<usize, StoreError> {
    if ids.is_empty() {
        return Ok(0);
    }
    let stamp = now();
    let mut tx = store
        .pool()
        .await?
        .begin()
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let mut marked = 0usize;
    for id in ids {
        let (status, object_id, origin) = match meta_of(&mut tx, id).await {
            Ok(meta) => meta,
            // A claim that vanished between snapshot and mark is not markable;
            // skip it rather than aborting the batch. Counted as examined by
            // the caller, not marked.
            Err(StoreError::NotFound) => continue,
            Err(other) => return Err(other),
        };
        let Some(status) = ClaimStatus::parse(&status) else {
            continue;
        };
        if !matches!(status, ClaimStatus::Candidate | ClaimStatus::Confirmed) {
            // A race flipped the status out of the legal set — skip, do not
            // abort. The per-claim `mark_stale` would have returned `Conflict`.
            continue;
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
        marked += 1;
    }
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(marked)
}
