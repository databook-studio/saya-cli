use crate::{SqliteStateStore, StoreError};
use saya_types::JobId;

impl SqliteStateStore {
    /// Durably requests cancellation; true means this is the first request.
    /// The live runtime observes this intent and owns cancellation and joining.
    /// This never claims that a process stopped or a job reached terminal state.
    pub async fn request_coding_cancel(&self, job: &JobId) -> Result<bool, StoreError> {
        let mut tx = self
            .pool()
            .await?
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM coding_jobs WHERE job_id=?")
                .bind(job.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| StoreError::Unavailable)?;
        let state = state.ok_or(StoreError::NotFound)?;
        if matches!(state.as_str(), "completed" | "failed" | "cancelled") {
            return Ok(false);
        }
        let inserted = sqlx::query(
            "INSERT INTO coding_cancel_requests(job_id) VALUES (?) ON CONFLICT DO NOTHING",
        )
        .bind(job.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        Ok(inserted.rows_affected() == 1)
    }
    pub async fn coding_cancel_requested(&self, job: &JobId) -> Result<bool, StoreError> {
        let present: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM coding_cancel_requests WHERE job_id=?)",
        )
        .bind(job.as_str())
        .fetch_one(self.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        Ok(present != 0)
    }
}
