use saya_types::{BudgetLimitPolicyV1, JobId, JobState, Sha256Digest};
use sqlx::{Row, Sqlite, Transaction};

use super::records::state;
use crate::{SqliteStateStore, StoreError};

/// A policy row read back from the store, already revalidated: the JSON
/// parses into a valid policy, the stored bytes are its canonical encoding,
/// and the recorded digest matches the policy's own digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBudgetPolicy {
    /// The decoded policy.
    pub policy: BudgetLimitPolicyV1,
    /// The recorded content-identity digest of [`Self::policy`].
    pub policy_sha256: Sha256Digest,
}

/// Outcome of [`SqliteStateStore::record_coding_budget_policy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingBudgetPolicyAppend {
    /// No row existed and the policy was inserted; the job's policy is now
    /// fixed for its lifetime.
    Recorded,
    /// A row for this job already existed with the identical policy digest
    /// and passes read-path validation; the resume proof.
    AlreadyPresentSame,
}

impl SqliteStateStore {
    /// Records the job's budget policy exactly once. The job must exist and
    /// be Running. On an existing row a digest match is the resume proof
    /// (`AlreadyPresentSame`); a mismatch is a typed `PolicyConflict` refusal
    /// that leaves the original row untouched. Insert-once: there is no
    /// update or delete path. The first insert is refused with
    /// [`StoreError::Conflict`] when the Running job already has budget
    /// reservations: history without an authenticated policy is never
    /// adopted.
    pub async fn record_coding_budget_policy(
        &self,
        job_id: &JobId,
        policy: &BudgetLimitPolicyV1,
    ) -> Result<CodingBudgetPolicyAppend, StoreError> {
        let digest = policy.digest();
        let json = policy
            .canonical_json_bytes()
            .map_err(|_| StoreError::Invalid)?;
        let json = String::from_utf8(json).map_err(|_| StoreError::Invalid)?;
        let pool = self.pool().await?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let outcome = match require_running_job(&mut transaction, job_id).await {
            Ok(()) => {
                let existing_row: Option<sqlx::sqlite::SqliteRow> = sqlx::query(
                    "SELECT policy_sha256, policy_json FROM coding_budget_policies WHERE job_id=?",
                )
                .bind(job_id.as_str())
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| StoreError::Unavailable)?;
                if existing_row.is_some() {
                    insert_policy(&mut transaction, job_id, &digest, &json).await
                } else {
                    let reservations: i64 = sqlx::query_scalar(
                        "SELECT COUNT(*) FROM coding_budget_reservations WHERE job_id=?",
                    )
                    .bind(job_id.as_str())
                    .fetch_one(&mut *transaction)
                    .await
                    .map_err(|_| StoreError::Unavailable)?;
                    if reservations > 0 {
                        Err(StoreError::Conflict)
                    } else {
                        insert_policy(&mut transaction, job_id, &digest, &json).await
                    }
                }
            }
            Err(error) => Err(error),
        };
        finish(transaction, outcome).await
    }

    /// Reads back the recorded policy, revalidating the JSON and its recorded
    /// digest. A tampered or non-canonical row is a typed corruption error,
    /// never silently accepted.
    pub async fn get_coding_budget_policy(
        &self,
        job_id: &JobId,
    ) -> Result<Option<StoredBudgetPolicy>, StoreError> {
        let row = sqlx::query(
            "SELECT policy_sha256, policy_json FROM coding_budget_policies WHERE job_id=?",
        )
        .bind(job_id.as_str())
        .fetch_optional(self.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        row.as_ref().map(stored_policy).transpose()
    }
}

async fn require_running_job(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &JobId,
) -> Result<(), StoreError> {
    let job_state: Option<String> =
        sqlx::query_scalar("SELECT state FROM coding_jobs WHERE job_id=?")
            .bind(job_id.as_str())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    if job_state.as_deref().map(state).transpose()? != Some(JobState::Running) {
        return Err(StoreError::Conflict);
    }
    Ok(())
}

async fn insert_policy(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &JobId,
    digest: &Sha256Digest,
    json: &str,
) -> Result<CodingBudgetPolicyAppend, StoreError> {
    let inserted = sqlx::query(
        "INSERT INTO coding_budget_policies(job_id, policy_sha256, policy_json) VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(job_id.as_str())
    .bind(digest.as_str())
    .bind(json)
    .execute(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if inserted.rows_affected() == 1 {
        return Ok(CodingBudgetPolicyAppend::Recorded);
    }
    let stored =
        sqlx::query("SELECT policy_sha256, policy_json FROM coding_budget_policies WHERE job_id=?")
            .bind(job_id.as_str())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?
            .ok_or(StoreError::Invalid)?;
    let stored = stored_policy(&stored)?;
    if stored.policy_sha256 == *digest {
        Ok(CodingBudgetPolicyAppend::AlreadyPresentSame)
    } else {
        Err(StoreError::PolicyConflict)
    }
}

fn stored_policy(row: &sqlx::sqlite::SqliteRow) -> Result<StoredBudgetPolicy, StoreError> {
    let recorded = Sha256Digest::parse(
        &row.try_get::<String, _>("policy_sha256")
            .map_err(|_| StoreError::Invalid)?,
    )
    .map_err(|_| StoreError::Invalid)?;
    let json: String = row
        .try_get("policy_json")
        .map_err(|_| StoreError::Invalid)?;
    validate_stored_policy(recorded, &json)
}

/// The one validation shared by the read path and the record retry path: the
/// JSON must parse through the revalidating deserializer, the stored bytes
/// must equal the canonical encoding, and the recorded digest must be the
/// policy's own digest. Any failure is typed corruption, never a resume
/// proof.
fn validate_stored_policy(
    recorded: Sha256Digest,
    json: &str,
) -> Result<StoredBudgetPolicy, StoreError> {
    let policy =
        BudgetLimitPolicyV1::from_json_bytes(json.as_bytes()).map_err(|_| StoreError::Invalid)?;
    if policy
        .canonical_json_bytes()
        .map_err(|_| StoreError::Invalid)?
        .as_slice()
        != json.as_bytes()
    {
        return Err(StoreError::Invalid);
    }
    if policy.digest() != recorded {
        return Err(StoreError::Invalid);
    }
    Ok(StoredBudgetPolicy {
        policy,
        policy_sha256: recorded,
    })
}

async fn finish(
    transaction: Transaction<'_, Sqlite>,
    outcome: Result<CodingBudgetPolicyAppend, StoreError>,
) -> Result<CodingBudgetPolicyAppend, StoreError> {
    match outcome {
        Ok(CodingBudgetPolicyAppend::Recorded) => {
            transaction
                .commit()
                .await
                .map_err(|_| StoreError::Unavailable)?;
            Ok(CodingBudgetPolicyAppend::Recorded)
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
