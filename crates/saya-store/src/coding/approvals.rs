use std::num::NonZeroU64;

use saya_types::{
    ApprovalDisposition, ApprovalRequestV1, ApprovalScope, CodingEventKind, CodingEventV1,
    JobState, Sha256Digest,
};
use sqlx::{Row, Sqlite, Transaction};

use super::{
    approval_records::{pending_request, sequence},
    events::{CodingEventAppend, append_event_in_tx},
    records::state,
};
use crate::{SqliteStateStore, StoreError};

const MAX_PENDING_APPROVAL_REQUESTS: i64 = 128;
const MAX_APPROVAL_REQUESTS: i64 = 10_000;

/// Coordinator-owned facts checked against one pending request at consumption.
/// They are not an approval, and neither this context nor a store receipt permits an effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingApprovalConsumptionContext {
    pub(crate) policy_sha256: Sha256Digest,
    pub(crate) revocation_epoch: u64,
    pub(crate) actual_scope: ApprovalScope,
}

impl CodingApprovalConsumptionContext {
    pub fn new(
        policy_sha256: Sha256Digest,
        revocation_epoch: u64,
        actual_scope: ApprovalScope,
    ) -> Self {
        Self {
            policy_sha256,
            revocation_epoch,
            actual_scope,
        }
    }
}

impl SqliteStateStore {
    pub async fn record_coding_approval_request(
        &self,
        event: CodingEventV1,
    ) -> Result<CodingEventAppend, StoreError> {
        let CodingEventKind::ApprovalRequested { request } = event.event() else {
            return Err(StoreError::Invalid);
        };
        let request = request.clone();
        let pool = self.pool().await?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let outcome = match append_event_in_tx(&mut transaction, event).await {
            Ok(CodingEventAppend::AlreadyPresent(sequence)) => {
                Ok(CodingEventAppend::AlreadyPresent(sequence))
            }
            Ok(CodingEventAppend::Appended(sequence)) => admit_request(&mut transaction, &request)
                .await
                .map(|()| CodingEventAppend::Appended(sequence)),
            Err(error) => Err(error),
        };
        finish(transaction, outcome).await
    }

    pub async fn close_coding_approval(
        &self,
        event: CodingEventV1,
    ) -> Result<CodingEventAppend, StoreError> {
        let CodingEventKind::ApprovalClosed {
            request_sequence,
            disposition,
        } = event.event()
        else {
            return Err(StoreError::Invalid);
        };
        if matches!(disposition, ApprovalDisposition::Consumed { .. }) {
            return Err(StoreError::Invalid);
        }
        let request_sequence = *request_sequence;
        let pool = self.pool().await?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let outcome = match append_event_in_tx(&mut transaction, event.clone()).await {
            Ok(CodingEventAppend::AlreadyPresent(sequence)) => {
                Ok(CodingEventAppend::AlreadyPresent(sequence))
            }
            Ok(CodingEventAppend::Appended(sequence)) => {
                close_pending_request(&mut transaction, &event, request_sequence, sequence)
                    .await
                    .map(|()| CodingEventAppend::Appended(sequence))
            }
            Err(error) => Err(error),
        };
        finish(transaction, outcome).await
    }
}

async fn finish(
    transaction: Transaction<'_, Sqlite>,
    outcome: Result<CodingEventAppend, StoreError>,
) -> Result<CodingEventAppend, StoreError> {
    match outcome {
        Ok(append) => {
            transaction
                .commit()
                .await
                .map_err(|_| StoreError::Unavailable)?;
            Ok(append)
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

async fn admit_request(
    transaction: &mut Transaction<'_, Sqlite>,
    request: &ApprovalRequestV1,
) -> Result<(), StoreError> {
    let job_state: Option<String> =
        sqlx::query_scalar("SELECT state FROM coding_jobs WHERE job_id=?")
            .bind(request.job_id().as_str())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    if job_state.as_deref().map(state).transpose()? != Some(JobState::Running) {
        return Err(StoreError::Conflict);
    }
    match (request.agent_id(), request.invocation_id()) {
        (None, None) => {}
        (Some(agent_id), Some(invocation_id)) => {
            let row = sqlx::query(
                "SELECT current_invocation_id, state FROM coding_children WHERE job_id=? AND agent_id=?",
            )
            .bind(request.job_id().as_str())
            .bind(agent_id.as_str())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?
            .ok_or(StoreError::NotFound)?;
            let current: String = row
                .try_get("current_invocation_id")
                .map_err(|_| StoreError::Invalid)?;
            let child_state: String = row.try_get("state").map_err(|_| StoreError::Invalid)?;
            if current != invocation_id.as_str()
                || !matches!(state(&child_state)?, JobState::Queued | JobState::Running)
            {
                return Err(StoreError::Conflict);
            }
        }
        _ => return Err(StoreError::Invalid),
    }
    check_request_counts(transaction, request.job_id().as_str()).await?;
    sqlx::query("INSERT INTO coding_approval_requests(job_id, request_sequence) VALUES (?, ?)")
        .bind(request.job_id().as_str())
        .bind(sequence(request.request_sequence())?)
        .execute(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

async fn check_request_counts(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &str,
) -> Result<(), StoreError> {
    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM coding_approval_requests WHERE job_id=?")
            .bind(job_id)
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM coding_approval_requests WHERE job_id=? AND closed_by_sequence IS NULL",
    )
    .bind(job_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if total >= MAX_APPROVAL_REQUESTS || pending >= MAX_PENDING_APPROVAL_REQUESTS {
        return Err(StoreError::LimitExceeded);
    }
    Ok(())
}

async fn close_pending_request(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
    request_sequence: NonZeroU64,
    closure_sequence: NonZeroU64,
) -> Result<(), StoreError> {
    let request = pending_request(transaction, event.job_id(), request_sequence).await?;
    if event.agent_id() != request.agent_id() || event.invocation_id() != request.invocation_id() {
        return Err(StoreError::Conflict);
    }
    let changed = sqlx::query(
        "UPDATE coding_approval_requests SET closed_by_sequence=? WHERE job_id=? AND request_sequence=? AND closed_by_sequence IS NULL",
    )
    .bind(sequence(closure_sequence)?)
    .bind(event.job_id().as_str())
    .bind(sequence(request_sequence)?)
    .execute(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if changed.rows_affected() != 1 {
        return Err(StoreError::Conflict);
    }
    Ok(())
}
