use saya_types::{JobId, ManagedCheckoutId, Sha256Digest};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, Transaction};

use crate::{SqliteStateStore, StoreError};

/// Upper bound on accepted plan-manifest bytes: 8 MiB.
pub const MAX_CHECKOUT_PLAN_BYTES: usize = 8 * 1024 * 1024;

/// A plan-manifest row read back from the store, already revalidated: the
/// recorded digest equals sha256 over the stored bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredCheckoutPlan {
    /// The recorded content-identity digest of [`Self::plan_bytes`].
    pub plan_sha256: Sha256Digest,
    /// The stored plan-manifest bytes.
    pub plan_bytes: Vec<u8>,
}

impl std::fmt::Debug for StoredCheckoutPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredCheckoutPlan")
            .field("plan_sha256", &self.plan_sha256)
            .field("plan_bytes_len", &self.plan_bytes.len())
            .finish()
    }
}

/// Outcome of [`SqliteStateStore::record_checkout_plan_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutPlanAppend {
    /// No row existed and the manifest bytes were inserted under the digest.
    Recorded,
    /// A row already existed with the identical digest and passes read-path
    /// validation; the resume proof.
    AlreadyPresentSame,
}

impl SqliteStateStore {
    /// Persists the plan-manifest bytes for a managed checkout exactly once,
    /// under `BEGIN IMMEDIATE`. The checkout row must exist for the job
    /// ([`StoreError::NotFound`] otherwise). Bytes beyond
    /// [`MAX_CHECKOUT_PLAN_BYTES`] are [`StoreError::Invalid`], and the
    /// caller-supplied digest must equal the store-computed sha256 of the
    /// bytes, so bytes can never be bound to a foreign digest. An existing row
    /// with the same digest is `AlreadyPresentSame`; a different digest is a
    /// typed [`StoreError::Conflict`] that leaves the original row untouched.
    pub async fn record_checkout_plan_bytes(
        &self,
        job_id: &JobId,
        checkout_id: &ManagedCheckoutId,
        plan_sha256: &Sha256Digest,
        plan_bytes: &[u8],
    ) -> Result<CheckoutPlanAppend, StoreError> {
        if plan_bytes.len() > MAX_CHECKOUT_PLAN_BYTES || plan_digest(plan_bytes) != *plan_sha256 {
            return Err(StoreError::Invalid);
        }
        let pool = self.pool().await?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let outcome = match require_checkout_row(&mut transaction, job_id, checkout_id).await {
            Ok(()) => {
                insert_plan(
                    &mut transaction,
                    job_id,
                    checkout_id,
                    plan_sha256,
                    plan_bytes,
                )
                .await
            }
            Err(error) => Err(error),
        };
        finish(transaction, outcome).await
    }

    /// Reads back the stored plan bytes for a managed checkout, revalidating
    /// that the recorded digest is sha256 over the stored bytes. A corrupt or
    /// tampered row is [`StoreError::Invalid`], never silently accepted.
    pub async fn get_checkout_plan_bytes(
        &self,
        job_id: &JobId,
        checkout_id: &ManagedCheckoutId,
    ) -> Result<Option<StoredCheckoutPlan>, StoreError> {
        let row = sqlx::query(
            "SELECT plan_sha256, plan_bytes FROM coding_checkout_plans WHERE job_id=? AND managed_checkout_id=?",
        )
        .bind(job_id.as_str())
        .bind(checkout_id.as_str())
        .fetch_optional(self.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        row.as_ref().map(stored_plan).transpose()
    }
}

fn plan_digest(bytes: &[u8]) -> Sha256Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Sha256Digest::parse(&hex).expect("a sha256 hex encoding parses as a digest")
}

async fn require_checkout_row(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &JobId,
    checkout_id: &ManagedCheckoutId,
) -> Result<(), StoreError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM coding_managed_checkouts WHERE job_id=? AND checkout_id=?)",
    )
    .bind(job_id.as_str())
    .bind(checkout_id.as_str())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if !exists {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

async fn insert_plan(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &JobId,
    checkout_id: &ManagedCheckoutId,
    digest: &Sha256Digest,
    bytes: &[u8],
) -> Result<CheckoutPlanAppend, StoreError> {
    let inserted = sqlx::query(
        "INSERT INTO coding_checkout_plans(job_id, managed_checkout_id, plan_sha256, plan_bytes) VALUES (?, ?, ?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(job_id.as_str())
    .bind(checkout_id.as_str())
    .bind(digest.as_str())
    .bind(bytes)
    .execute(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if inserted.rows_affected() == 1 {
        return Ok(CheckoutPlanAppend::Recorded);
    }
    let existing = sqlx::query(
        "SELECT plan_sha256, plan_bytes FROM coding_checkout_plans WHERE job_id=? AND managed_checkout_id=?",
    )
    .bind(job_id.as_str())
    .bind(checkout_id.as_str())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    let stored = stored_plan(&existing)?;
    if stored.plan_sha256 == *digest {
        Ok(CheckoutPlanAppend::AlreadyPresentSame)
    } else {
        Err(StoreError::Conflict)
    }
}

fn stored_plan(row: &sqlx::sqlite::SqliteRow) -> Result<StoredCheckoutPlan, StoreError> {
    let recorded = Sha256Digest::parse(
        &row.try_get::<String, _>("plan_sha256")
            .map_err(|_| StoreError::Invalid)?,
    )
    .map_err(|_| StoreError::Invalid)?;
    let bytes: Vec<u8> = row.try_get("plan_bytes").map_err(|_| StoreError::Invalid)?;
    if bytes.len() > MAX_CHECKOUT_PLAN_BYTES || plan_digest(&bytes) != recorded {
        return Err(StoreError::Invalid);
    }
    Ok(StoredCheckoutPlan {
        plan_sha256: recorded,
        plan_bytes: bytes,
    })
}

async fn finish(
    transaction: Transaction<'_, Sqlite>,
    outcome: Result<CheckoutPlanAppend, StoreError>,
) -> Result<CheckoutPlanAppend, StoreError> {
    match outcome {
        Ok(CheckoutPlanAppend::Recorded) => {
            transaction
                .commit()
                .await
                .map_err(|_| StoreError::Unavailable)?;
            Ok(CheckoutPlanAppend::Recorded)
        }
        Ok(already_present) => {
            transaction
                .rollback()
                .await
                .map_err(|_| StoreError::Unavailable)?;
            Ok(already_present)
        }
        Err(error) => {
            transaction
                .rollback()
                .await
                .map_err(|_| StoreError::Unavailable)?;
            Err(error)
        }
    }
}
