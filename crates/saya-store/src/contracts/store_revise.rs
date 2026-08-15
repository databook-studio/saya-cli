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
use saya_types::{ClaimId, ClaimPayload, ClaimStatus, SchemaFingerprint, Table};

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
    // Edit accepts Stale as well as Candidate and Confirmed: editing is how a
    // user repairs a claim whose referenced column was renamed. Refusing Stale
    // left such a claim unrepairable — confirm was a no-op and edit a refusal.
    // Edit re-runs admission and recomputes the dedup key; with no live table
    // it stores name-only snapshots (unknown, not Current), so the claim
    // unblocks to Confirmed and a later reconcile can promote it.
    let legal = matches!(
        status,
        ClaimStatus::Candidate | ClaimStatus::Confirmed | ClaimStatus::Stale
    );
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

/// Reconfirm a claim against a live table, rewriting its fingerprint and
/// referenced-column snapshots in the same transaction as the status flip.
///
/// This is the schema-aware counterpart to [`super::store_transitions::confirm_claim`].
/// A plain confirm used to flip a Stale claim to Confirmed but leave the stored
/// fingerprint untouched, so the next read recomputed the digest, found it still
/// differed, and returned Stale again — a silent no-op the user could not repair.
/// Revalidation breaks that loop: it stores `of_table(live)` and fresh snapshots,
/// so the next read classifies the claim Current against the schema it was
/// confirmed against.
///
/// Where a referenced column is absent from `live_table`, refuse with
/// [`StoreError::Conflict`] and change nothing: reviving a claim whose
/// dependency vanished would read Current against a schema it no longer
/// matches, which is worse than the no-op the bug left. The status, fingerprint,
/// snapshots, and audit event move together or not at all — a refusal leaves the
/// claim exactly as it was.
///
/// Legal from `Candidate | Stale | Contradicted` — the same set a plain confirm
/// accepted. A Confirmed claim is already confirmed and is a `Conflict`.
pub(crate) async fn revalidate_claim(
    store: &SqliteStateStore,
    id: &ClaimId,
    live_table: &Table,
) -> Result<StoredClaim, StoreError> {
    // Read the claim before the transaction for its payload and object — the
    // data needed to recompute the digest and snapshots, which the status-only
    // `meta_of` helper does not return. The transaction re-reads the status
    // for the legal-transition check, so a concurrent status change is still
    // caught there rather than racing this read.
    let claim = store_reads::get_claim(store, id)
        .await?
        .ok_or(StoreError::NotFound)?;
    let Some(payload) = claim.payload.clone() else {
        // A tombstoned (forgotten) claim has no payload and no referenced
        // columns to snapshot. It is not revalidatable.
        return Err(StoreError::Conflict);
    };
    // Every referenced column must still exist in the live table. A claim
    // cannot be reconfirmed against a schema that dropped something it
    // depends on — the user must edit or forget it instead.
    if has_missing_referenced_column(&payload, live_table) {
        return Err(StoreError::Conflict);
    }
    let fingerprint = SchemaFingerprint::of_table(claim.object.kind(), live_table);
    let referenced = payload.referenced_column_snapshots(live_table);
    let referenced_serialized =
        serde_json::to_string(&referenced).map_err(|_| StoreError::Invalid)?;
    if redact(&referenced_serialized) != referenced_serialized {
        return Err(StoreError::Invalid);
    }
    admission::check(&referenced_serialized)?;
    let stamp = now();
    let mut tx = store
        .pool()
        .await?
        .begin()
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let (status, object_id, origin) = meta_of(&mut tx, id).await?;
    let status = ClaimStatus::parse(&status).ok_or(StoreError::Invalid)?;
    let legal = matches!(
        status,
        ClaimStatus::Candidate | ClaimStatus::Stale | ClaimStatus::Contradicted
    );
    if !legal {
        return Err(StoreError::Conflict);
    }
    sqlx::query("UPDATE contract_claims SET status='confirmed', schema_fingerprint=?, referenced_columns_json=?, last_verified_unix_ms=?, updated_unix_ms=? WHERE id=?")
        .bind(fingerprint.as_str())
        .bind(&referenced_serialized)
        .bind(stamp)
        .bind(stamp)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("INSERT INTO contract_events(claim_id, object_id, event, from_status, to_status, origin, created_unix_ms) VALUES (?, ?, 'confirmed', ?, 'confirmed', ?, ?)")
        .bind(id.as_str())
        .bind(&object_id)
        .bind(status.as_str())
        .bind(&origin)
        .bind(stamp)
        .execute(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    store_reads::get_claim(store, id)
        .await?
        .ok_or(StoreError::NotFound)
}

/// True when a column the claim depends on is absent from `live_table`. The
/// snapshot builder skips such columns; this check turns that skip into a
/// refusal so revalidation never silently drops a dependency.
fn has_missing_referenced_column(payload: &ClaimPayload, live_table: &Table) -> bool {
    payload.referenced_columns().iter().any(|name| {
        !live_table
            .columns
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case(name))
    })
}
