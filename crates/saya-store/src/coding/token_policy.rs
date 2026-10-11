use crate::{SqliteStateStore, StoreError};
use saya_types::{JobId, TokenBudgetPolicyV1};
use sqlx::Row;
impl SqliteStateStore {
    pub async fn record_coding_token_policy(
        &self,
        job: &JobId,
        policy: &TokenBudgetPolicyV1,
    ) -> Result<(), StoreError> {
        let mut tx = self
            .pool()
            .await?
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let row = sqlx::query(
            "SELECT policy_json,policy_sha256 FROM coding_token_policies WHERE job_id=?",
        )
        .bind(job.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        if let Some(row) = row {
            if decode(&row)? != *policy {
                return Err(StoreError::PolicyConflict);
            }
        } else {
            let state: Option<String> =
                sqlx::query_scalar("SELECT state FROM coding_jobs WHERE job_id=?")
                    .bind(job.as_str())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|_| StoreError::Unavailable)?;
            let history: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM coding_budget_reservations WHERE job_id=?",
            )
            .bind(job.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
            if state.as_deref() != Some("running") || history != 0 {
                return Err(StoreError::Conflict);
            }
            sqlx::query("INSERT INTO coding_token_policies(job_id,policy_json,policy_sha256) VALUES (?,?,?)")
                .bind(job.as_str()).bind(policy.canonical_json_bytes()).bind(policy.digest().as_str()).execute(&mut *tx).await.map_err(|_|StoreError::Unavailable)?;
        }
        tx.commit().await.map_err(|_| StoreError::Unavailable)
    }
    pub async fn get_coding_token_policy(
        &self,
        job: &JobId,
    ) -> Result<Option<TokenBudgetPolicyV1>, StoreError> {
        let row = sqlx::query(
            "SELECT policy_json,policy_sha256 FROM coding_token_policies WHERE job_id=?",
        )
        .bind(job.as_str())
        .fetch_optional(self.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        row.as_ref().map(decode).transpose()
    }
}
fn decode(row: &sqlx::sqlite::SqliteRow) -> Result<TokenBudgetPolicyV1, StoreError> {
    let bytes: Vec<u8> = row
        .try_get("policy_json")
        .map_err(|_| StoreError::Invalid)?;
    let digest: String = row
        .try_get("policy_sha256")
        .map_err(|_| StoreError::Invalid)?;
    let policy = TokenBudgetPolicyV1::from_json_bytes(&bytes).map_err(|_| StoreError::Invalid)?;
    if policy.digest().as_str() != digest {
        return Err(StoreError::Invalid);
    }
    Ok(policy)
}
