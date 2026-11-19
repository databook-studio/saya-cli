//! Recovery inspection never reconstructs grants or executes an effect.
use super::runtime::{CodingRuntimeAuthority, authority_in_tx};
use crate::{SqliteStateStore, StoreError};
use saya_types::{CodingEventKind, CodingEventV1, CodingFailureClass, JobId, JobState};
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
    /// How the job's last run ended, from the terminal event it appended.
    /// `None` while no run has terminated.
    pub failure_class: Option<CodingFailureClass>,
}
impl CodingRecoveryStatus {
    pub fn clear_for_resume(&self) -> bool {
        // A completed run may be continued explicitly, like a paused one: the
        // same recovery facts decide, and a completed job has none open.
        let resumable = matches!(
            self.state,
            JobState::Running | JobState::Paused | JobState::Completed
        )
            // A run the provider failed ended for a reason outside the job's
            // own work, with its retries spent: continuing it replays no
            // effect and repeats no step. Any other stop is the job's own
            // answer and is not a resume but a new job.
            || (self.state == JobState::Failed
                && self.failure_class == Some(CodingFailureClass::ProviderError));
        resumable
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
    // references. An unknown effect cannot be treated as a measured refund;
    // unknown model-attempt usage alone is not unresolved.
    // A reservation with a recorded reconciliation fact is resolved: the fact
    // is durable and never refunds the reservation's charges.
    let unresolved = format!(
        "SELECT COUNT(*) FROM coding_budget_reservations r LEFT JOIN coding_events e ON e.job_id=r.job_id AND e.sequence=r.settled_by_sequence WHERE r.job_id=? AND r.reconciled_by_sequence IS NULL AND {}",
        super::unresolved::unresolved("e.event_json")
    );
    let unresolved_reservations = count(tx, job, &unresolved).await?;
    let pending_approvals = count(tx, job, "SELECT COUNT(*) FROM coding_approval_requests WHERE job_id=? AND closed_by_sequence IS NULL").await?;
    let unresolved_checkouts = count(tx, job, "SELECT COUNT(*) FROM coding_checkout_operations o LEFT JOIN coding_events e ON e.job_id=o.job_id AND e.sequence=o.outcome_sequence WHERE o.job_id=? AND COALESCE(json_extract(e.event_json,'$.event.operation.state'),'') NOT IN ('completed','failed')").await?;
    let failure_class = failure_class_in_tx(tx, job).await?;
    Ok(CodingRecoveryStatus {
        state,
        last_sequence,
        active_children,
        unresolved_reservations,
        pending_approvals,
        unresolved_checkouts,
        authority: authority_in_tx(tx, job).await?,
        failure_class,
    })
}

/// The class the job's last run ended with. A resumed run appends its own
/// terminal event, so this always describes the run a caller would continue.
async fn failure_class_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    job: &JobId,
) -> Result<Option<CodingFailureClass>, StoreError> {
    let bytes: Option<String> = sqlx::query_scalar(
        "SELECT event_json FROM coding_events WHERE job_id=? AND json_extract(event_json,'$.agent_id') IS NULL AND json_extract(event_json,'$.event.type')='run_terminated' ORDER BY sequence DESC LIMIT 1",
    )
    .bind(job.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let event =
        CodingEventV1::from_json_bytes(bytes.as_bytes()).map_err(|_| StoreError::Invalid)?;
    Ok(match event.event() {
        CodingEventKind::RunTerminated { failure, .. } => Some(failure.class),
        _ => None,
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
