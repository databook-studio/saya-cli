use crate::contracts::admission;
use crate::contracts::keys::{claim_id, deduplication_key};
use crate::contracts::records::{
    ClaimEvidence, MAX_CLAIM_PAYLOAD_BYTES, MAX_CLAIMS_PER_OBJECT, MAX_EVIDENCE_PER_CLAIM,
    ProposeClaim, ProposeOutcome,
};
use crate::contracts::store::{now, upsert_object_in_tx};
use crate::{SqliteStateStore, StoreError, redact};
use saya_types::{CLAIM_PAYLOAD_VERSION, ClaimId, ClaimStatus};

pub(crate) async fn propose_claim(
    store: &SqliteStateStore,
    request: ProposeClaim,
) -> Result<ProposeOutcome, StoreError> {
    if !matches!(
        request.initial_status,
        ClaimStatus::Candidate | ClaimStatus::Confirmed
    ) {
        return Err(StoreError::Invalid);
    }
    if matches!(request.initial_status, ClaimStatus::Confirmed)
        && !request.origin.may_confirm_directly()
    {
        return Err(StoreError::Invalid);
    }
    let serialized = serde_json::to_string(&request.payload).map_err(|_| StoreError::Invalid)?;
    if serialized.len() > MAX_CLAIM_PAYLOAD_BYTES {
        return Err(StoreError::LimitExceeded);
    }
    // A claim is a short business statement a human agreed to keep; a credential
    // marker inside one means the extraction is wrong — laundering it into storage
    // would hide the bug and keep the secret. Refuse rather than redact-and-store.
    if redact(&serialized) != serialized {
        return Err(StoreError::Invalid);
    }
    admission::check(&serialized)?;
    let stamp = now();
    let mut tx = store
        .pool()
        .await?
        .begin()
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let object_id =
        upsert_object_in_tx(&mut tx, &request.object, &request.fingerprint, stamp).await?;
    let key = deduplication_key(&request.payload, &serialized);
    if let Some((existing_id, existing_status)) = sqlx::query_as::<_, (String, String)>(
        "SELECT id, status FROM contract_claims WHERE object_id=? AND deduplication_key=?",
    )
    .bind(object_id.as_str())
    .bind(key.as_str())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| StoreError::Unavailable)?
    {
        let id = ClaimId::parse(&existing_id).map_err(|_| StoreError::Invalid)?;
        let status = ClaimStatus::parse(&existing_status).ok_or(StoreError::Invalid)?;
        append_evidence(&mut tx, &existing_id, request.evidence.as_ref()).await?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        store.secure_files()?;
        return Ok(ProposeOutcome::Duplicate { id, status });
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contract_claims WHERE object_id=?")
        .bind(object_id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    if count >= MAX_CLAIMS_PER_OBJECT as i64 {
        return Err(StoreError::LimitExceeded);
    }
    let claim_id = claim_id(&object_id, &key)?;
    let referenced = serde_json::to_string(&request.payload.referenced_columns())
        .map_err(|_| StoreError::Invalid)?;
    sqlx::query("INSERT INTO contract_claims(id, object_id, claim_kind, payload_json, payload_version, origin, status, schema_fingerprint, referenced_columns_json, created_unix_ms, updated_unix_ms, last_verified_unix_ms, deduplication_key) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)")
        .bind(claim_id.as_str())
        .bind(object_id.as_str())
        .bind(request.payload.kind())
        .bind(&serialized)
        .bind(CLAIM_PAYLOAD_VERSION as i64)
        .bind(request.origin.as_str())
        .bind(request.initial_status.as_str())
        .bind(request.fingerprint.as_str())
        .bind(&referenced)
        .bind(stamp)
        .bind(stamp)
        .bind(key.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    append_evidence(&mut tx, claim_id.as_str(), request.evidence.as_ref()).await?;
    sqlx::query("INSERT INTO contract_events(claim_id, object_id, event, from_status, to_status, origin, created_unix_ms) VALUES (?, ?, 'proposed', NULL, ?, ?, ?)")
        .bind(claim_id.as_str())
        .bind(object_id.as_str())
        .bind(request.initial_status.as_str())
        .bind(request.origin.as_str())
        .bind(stamp)
        .execute(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(ProposeOutcome::Stored(claim_id))
}

async fn append_evidence(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    claim_id: &str,
    evidence: Option<&ClaimEvidence>,
) -> Result<(), StoreError> {
    let Some(evidence) = evidence else {
        return Ok(());
    };
    // Insert before pruning. The unique index makes a repeat of already-recorded
    // evidence a no-op, and pruning first would spend the claim's oldest row to
    // make space for an insert that never happens — quietly dropping support
    // every time the same observation is seen again at the cap.
    let inserted = sqlx::query("INSERT OR IGNORE INTO contract_evidence(claim_id, evidence_kind, session_id, turn_ordinal, observed_unix_ms) VALUES (?, ?, ?, ?, ?)")
        .bind(claim_id)
        .bind(evidence.kind.as_str())
        .bind(evidence.session_id.as_deref())
        .bind(evidence.turn_ordinal.map(|value| value as i64))
        .bind(evidence.observed_unix_ms)
        .execute(&mut **tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    if inserted.rows_affected() == 0 {
        return Ok(());
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contract_evidence WHERE claim_id=?")
        .bind(claim_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let excess = count - MAX_EVIDENCE_PER_CLAIM as i64;
    if excess > 0 {
        sqlx::query("DELETE FROM contract_evidence WHERE claim_id=? AND id IN (SELECT id FROM contract_evidence WHERE claim_id=? ORDER BY observed_unix_ms ASC, id ASC LIMIT ?)")
            .bind(claim_id)
            .bind(claim_id)
            .bind(excess)
            .execute(&mut **tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    }
    Ok(())
}
