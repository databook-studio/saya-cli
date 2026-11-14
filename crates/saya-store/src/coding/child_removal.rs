//! Taking back the registration of a child that was never scheduled.
use crate::{SqliteStateStore, StoreError};
use saya_types::{AgentId, InvocationId, JobId};
use sqlx::Row;

impl SqliteStateStore {
    /// Removes a child registered for a delegation that failed before the child
    /// could be scheduled, so no queued orphan is left for the job to join. The
    /// caller names the invocation it registered; the child must still be
    /// queued, with no result, completion or budget reservation recorded for
    /// it. Anything that ran refuses with `Conflict`: its records are
    /// append-only and stay. Events already appended for the child also stay.
    pub async fn remove_unscheduled_coding_child(
        &self,
        job_id: &JobId,
        agent_id: &AgentId,
        invocation_id: &InvocationId,
    ) -> Result<(), StoreError> {
        let mut tx = self
            .pool()
            .await?
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let row = sqlx::query("SELECT current_invocation_id, state, result_outcome FROM coding_children WHERE job_id=? AND agent_id=?")
            .bind(job_id.as_str())
            .bind(agent_id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?
            .ok_or(StoreError::NotFound)?;
        let current: String = row
            .try_get("current_invocation_id")
            .map_err(|_| StoreError::Invalid)?;
        let state: String = row.try_get("state").map_err(|_| StoreError::Invalid)?;
        let outcome: Option<String> = row
            .try_get("result_outcome")
            .map_err(|_| StoreError::Invalid)?;
        if current != invocation_id.as_str() || state != "queued" || outcome.is_some() {
            return Err(StoreError::Conflict);
        }
        let ran: i64 = sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM coding_completions WHERE job_id=? AND agent_id=?) + (SELECT COUNT(*) FROM coding_events WHERE job_id=? AND json_extract(event_json,'$.agent_id')=? AND json_extract(event_json,'$.event.type')='budget_reserved')")
            .bind(job_id.as_str())
            .bind(agent_id.as_str())
            .bind(job_id.as_str())
            .bind(agent_id.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        if ran != 0 {
            return Err(StoreError::Conflict);
        }
        let removed = sqlx::query("DELETE FROM coding_children WHERE job_id=? AND agent_id=? AND current_invocation_id=? AND state='queued'")
            .bind(job_id.as_str())
            .bind(agent_id.as_str())
            .bind(invocation_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        if removed.rows_affected() != 1 {
            return Err(StoreError::Conflict);
        }
        tx.commit().await.map_err(|_| StoreError::Unavailable)
    }
}
