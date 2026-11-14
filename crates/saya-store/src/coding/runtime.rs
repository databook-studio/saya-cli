use crate::{SqliteStateStore, StoreError};
use saya_types::{CodingEventKind, CodingEventV1, JobId, Sha256Digest};
use sqlx::{Row, Sqlite, Transaction};
use std::num::NonZeroU64;

/// Durable fence identity, never a stored permission grant. Trusted composition
/// supplies a freshly issued root policy and epoch after explicit user consent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingRuntimeAuthority {
    pub epoch: u64,
    pub policy_digest: Sha256Digest,
}
impl SqliteStateStore {
    /// Initial binding is idempotent for the exact same identity. It never renews.
    pub async fn bind_coding_runtime(
        &self,
        job: &JobId,
        authority: &CodingRuntimeAuthority,
    ) -> Result<(), StoreError> {
        let epoch = epoch(authority)?;
        let mut tx = self
            .pool()
            .await?
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        match authority_in_tx(&mut tx, job).await? {
            Some(current) if current != *authority => return Err(StoreError::Conflict),
            Some(_) => {}
            None => {
                sqlx::query("INSERT INTO coding_runtime_authority(job_id, epoch, policy_digest) VALUES (?, ?, ?)")
                    .bind(job.as_str()).bind(epoch).bind(authority.policy_digest.as_str())
                    .execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
            }
        }
        tx.commit().await.map_err(|_| StoreError::Unavailable)
    }
    /// Compare-and-swap renewal after a fresh authority decision. The recovery
    /// facts and event tail are checked in the same transaction as the fence.
    pub async fn renew_coding_runtime(
        &self,
        job: &JobId,
        previous: Option<&CodingRuntimeAuthority>,
        fresh: &CodingRuntimeAuthority,
        expected_last_sequence: u64,
    ) -> Result<super::CodingEventAppend, StoreError> {
        let epoch = epoch(fresh)?;
        if previous.is_some_and(|old| fresh.epoch <= old.epoch) {
            return Err(StoreError::Conflict);
        }
        let mut tx = self
            .pool()
            .await?
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let status = super::recovery::status_in_tx(&mut tx, job).await?;
        if status.authority.as_ref() != previous
            || status.last_sequence != expected_last_sequence
            || !status.clear_for_resume()
        {
            return Err(StoreError::Conflict);
        }
        sqlx::query("INSERT INTO coding_runtime_authority(job_id, epoch, policy_digest) VALUES (?, ?, ?) ON CONFLICT(job_id) DO UPDATE SET epoch=excluded.epoch, policy_digest=excluded.policy_digest")
            .bind(job.as_str()).bind(epoch).bind(fresh.policy_digest.as_str()).execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
        let sequence = expected_last_sequence
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .ok_or(StoreError::LimitExceeded)?;
        let event = CodingEventV1::new(
            sequence,
            job.clone(),
            None,
            None,
            CodingEventKind::RuntimeResumed {
                revocation_epoch: NonZeroU64::new(fresh.epoch).ok_or(StoreError::Invalid)?,
                policy_sha256: fresh.policy_digest.clone(),
            },
        )
        .map_err(|_| StoreError::Invalid)?;
        let append = super::events::append_event_in_tx(&mut tx, event).await?;
        sqlx::query("DELETE FROM coding_cancel_requests WHERE job_id=?")
            .bind(job.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        Ok(append)
    }
}
pub(super) async fn authority_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    job: &JobId,
) -> Result<Option<CodingRuntimeAuthority>, StoreError> {
    let row =
        sqlx::query("SELECT epoch, policy_digest FROM coding_runtime_authority WHERE job_id=?")
            .bind(job.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    row.map(|row| {
        let epoch: i64 = row.try_get("epoch").map_err(|_| StoreError::Invalid)?;
        let digest: String = row
            .try_get("policy_digest")
            .map_err(|_| StoreError::Invalid)?;
        let epoch = u64::try_from(epoch)
            .ok()
            .filter(|epoch| *epoch > 0)
            .ok_or(StoreError::Invalid)?;
        Ok(CodingRuntimeAuthority {
            epoch,
            policy_digest: Sha256Digest::parse(&digest).map_err(|_| StoreError::Invalid)?,
        })
    })
    .transpose()
}
pub(super) async fn check_fence(
    tx: &mut Transaction<'_, Sqlite>,
    job: &JobId,
    expected: Option<&CodingRuntimeAuthority>,
) -> Result<(), StoreError> {
    ensure_not_cancelled(tx, job).await?;
    if authority_in_tx(tx, job).await?.as_ref() != expected {
        return Err(StoreError::Conflict);
    }
    Ok(())
}
fn epoch(authority: &CodingRuntimeAuthority) -> Result<i64, StoreError> {
    i64::try_from(authority.epoch)
        .ok()
        .filter(|epoch| *epoch > 0)
        .ok_or(StoreError::Invalid)
}

pub(super) async fn ensure_not_cancelled(
    tx: &mut Transaction<'_, Sqlite>,
    job: &JobId,
) -> Result<(), StoreError> {
    let cancelled: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM coding_cancel_requests WHERE job_id=?)")
            .bind(job.as_str())
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    if cancelled != 0 {
        return Err(StoreError::Conflict);
    }
    Ok(())
}
