//! Recovery inspection never reconstructs grants or executes an effect.
use super::runtime::{CodingRuntimeAuthority, authority_in_tx};
use crate::{SqliteStateStore, StoreError};
use saya_types::{JobId, JobState};
use sqlx::{Sqlite, Transaction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingRecoveryStatus {
    pub state: JobState,
    pub last_sequence: u64,
    pub active_children: u64,
    pub unresolved_reservations: u64,
    pub pending_approvals: u64,
    pub unresolved_checkouts: u64,
    pub authority: Option<CodingRuntimeAuthority>,
}
impl CodingRecoveryStatus {
    pub fn clear_for_resume(&self) -> bool {
        matches!(self.state, JobState::Running | JobState::Paused)
            && self.active_children == 0
            && self.unresolved_reservations == 0
            && self.pending_approvals == 0
            && self.unresolved_checkouts == 0
    }
}
impl SqliteStateStore {
    pub async fn coding_recovery_status(
        &self,
        job: &JobId,
    ) -> Result<CodingRecoveryStatus, StoreError> {
        let mut tx = self
            .pool()
            .await?
            .begin()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let status = status_in_tx(&mut tx, job).await?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        Ok(status)
    }
}
pub(super) async fn status_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    job: &JobId,
) -> Result<CodingRecoveryStatus, StoreError> {
    let state: Option<String> = sqlx::query_scalar("SELECT state FROM coding_jobs WHERE job_id=?")
        .bind(job.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let state = super::records::state(&state.ok_or(StoreError::NotFound)?)?;
    let last_sequence = count(
        tx,
        job,
        "SELECT COALESCE(MAX(sequence),0) FROM coding_events WHERE job_id=?",
    )
    .await?;
    let active_children = count(tx, job, "SELECT COUNT(*) FROM coding_children WHERE job_id=? AND state NOT IN ('queued','completed','failed','cancelled')").await?;
    // Missing or unrecognized settlement state is unresolved, including corrupt
    // references. Unknown observations cannot be treated as a measured refund.
    let unresolved_reservations = count(tx, job, "SELECT COUNT(*) FROM coding_budget_reservations r LEFT JOIN coding_events e ON e.job_id=r.job_id AND e.sequence=r.settled_by_sequence WHERE r.job_id=? AND COALESCE(json_extract(e.event_json,'$.event.settlement.effect_state'),'') != 'settled'").await?;
    let pending_approvals = count(tx, job, "SELECT COUNT(*) FROM coding_approval_requests WHERE job_id=? AND closed_by_sequence IS NULL").await?;
    let unresolved_checkouts = count(tx, job, "SELECT COUNT(*) FROM coding_checkout_operations o LEFT JOIN coding_events e ON e.job_id=o.job_id AND e.sequence=o.outcome_sequence WHERE o.job_id=? AND COALESCE(json_extract(e.event_json,'$.event.operation.state'),'') NOT IN ('completed','failed')").await?;
    Ok(CodingRecoveryStatus {
        state,
        last_sequence,
        active_children,
        unresolved_reservations,
        pending_approvals,
        unresolved_checkouts,
        authority: authority_in_tx(tx, job).await?,
    })
}
async fn count(
    tx: &mut Transaction<'_, Sqlite>,
    job: &JobId,
    query: &str,
) -> Result<u64, StoreError> {
    let value: i64 = sqlx::query_scalar(query)
        .bind(job.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    u64::try_from(value).map_err(|_| StoreError::Invalid)
}
