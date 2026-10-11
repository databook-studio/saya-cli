use std::num::NonZeroU64;

use saya_types::{ApprovalDisposition, BudgetReservationV1, CodingEventKind, CodingEventV1};
use sqlx::{Row, Sqlite, Transaction};

use super::{
    approval_records::{closure_from_json, pending_request, request_from_json, sequence},
    approvals::CodingApprovalConsumptionContext,
    budget_records::reservation_from_json,
    budgets::admit_reservation,
    events::{CodingEventAppend, append_event_in_tx},
    records::now,
};
use crate::{SqliteStateStore, StoreError};

/// Durable audit result only. It never grants an external effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodingApprovalConsumptionReceipt {
    pub reservation_sequence: NonZeroU64,
    pub closure_sequence: NonZeroU64,
}

/// An idempotent consumption append outcome, not a capability decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodingApprovalConsumptionAppend {
    Recorded(CodingApprovalConsumptionReceipt),
    AlreadyPresent(CodingApprovalConsumptionReceipt),
}

impl SqliteStateStore {
    /// Records a reservation and its consumed closure atomically after the caller obtains
    /// fresh human/root consent. Stored requests and receipts never authorize execution.
    pub async fn consume_coding_approval(
        &self,
        reservation_event: CodingEventV1,
        closure_event: CodingEventV1,
        current_context: CodingApprovalConsumptionContext,
    ) -> Result<CodingApprovalConsumptionAppend, StoreError> {
        let CodingEventKind::BudgetReserved { reservation } = reservation_event.event() else {
            return Err(StoreError::Invalid);
        };
        let CodingEventKind::ApprovalClosed {
            request_sequence,
            disposition:
                ApprovalDisposition::Consumed {
                    reservation_sequence,
                },
        } = closure_event.event()
        else {
            return Err(StoreError::Invalid);
        };
        let reservation = reservation.clone();
        let request_sequence = *request_sequence;
        let reservation_sequence = *reservation_sequence;
        if reservation.reservation_sequence() != reservation_sequence
            || reservation_event.job_id() != closure_event.job_id()
            || reservation_event.sequence() != reservation_sequence
            || reservation_sequence.get().checked_add(1) != Some(closure_event.sequence().get())
        {
            return Err(StoreError::Invalid);
        }
        let receipt = CodingApprovalConsumptionReceipt {
            reservation_sequence,
            closure_sequence: closure_event.sequence(),
        };
        let pool = self.pool().await?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        super::runtime::ensure_not_cancelled(&mut transaction, reservation_event.job_id()).await?;
        if let Some(active) =
            super::runtime::authority_in_tx(&mut transaction, reservation_event.job_id()).await?
            && (active.epoch != current_context.revocation_epoch
                || active.policy_digest != current_context.policy_sha256)
        {
            return Err(StoreError::Conflict);
        }
        let outcome = match append_event_in_tx(&mut transaction, reservation_event).await {
            Err(error) => Err(error),
            Ok(reservation_append) => {
                match append_event_in_tx(&mut transaction, closure_event.clone()).await {
                    Err(error) => Err(error),
                    Ok(closure_append) => match (reservation_append, closure_append) {
                        (
                            CodingEventAppend::AlreadyPresent(_),
                            CodingEventAppend::AlreadyPresent(_),
                        ) => existing_pair(
                            &mut transaction,
                            closure_event.job_id(),
                            request_sequence,
                            receipt,
                        )
                        .await
                        .map(|()| CodingApprovalConsumptionAppend::AlreadyPresent(receipt)),
                        (CodingEventAppend::Appended(_), CodingEventAppend::Appended(_)) => {
                            consume_new_pair(
                                &mut transaction,
                                &reservation,
                                &closure_event,
                                request_sequence,
                                &current_context,
                            )
                            .await
                            .map(|()| CodingApprovalConsumptionAppend::Recorded(receipt))
                        }
                        _ => Err(StoreError::Conflict),
                    },
                }
            }
        };
        finish(transaction, outcome).await
    }
}

async fn consume_new_pair(
    transaction: &mut Transaction<'_, Sqlite>,
    reservation: &BudgetReservationV1,
    closure_event: &CodingEventV1,
    request_sequence: NonZeroU64,
    current_context: &CodingApprovalConsumptionContext,
) -> Result<(), StoreError> {
    let request = pending_request(transaction, reservation.job_id(), request_sequence).await?;
    if request.agent_id() != reservation.agent_id()
        || request.invocation_id() != reservation.invocation_id()
        || closure_event.agent_id() != request.agent_id()
        || closure_event.invocation_id() != request.invocation_id()
        || request.policy_sha256() != &current_context.policy_sha256
        || request.revocation_epoch() != current_context.revocation_epoch
        || request.scope() != &current_context.actual_scope
    {
        return Err(StoreError::Conflict);
    }
    let requested =
        i64::try_from(request.requested_at_unix_ms()).map_err(|_| StoreError::LimitExceeded)?;
    let expires =
        i64::try_from(request.expires_at_unix_ms()).map_err(|_| StoreError::LimitExceeded)?;
    let timestamp = now()?;
    if timestamp < requested || timestamp >= expires {
        return Err(StoreError::Conflict);
    }
    admit_reservation(transaction, reservation).await?;
    let changed = sqlx::query(
        "UPDATE coding_approval_requests SET closed_by_sequence=? WHERE job_id=? AND request_sequence=? AND closed_by_sequence IS NULL",
    )
    .bind(sequence(closure_event.sequence())?)
    .bind(reservation.job_id().as_str())
    .bind(sequence(request_sequence)?)
    .execute(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if changed.rows_affected() != 1 {
        return Err(StoreError::Conflict);
    }
    Ok(())
}

async fn existing_pair(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &saya_types::JobId,
    request_sequence: NonZeroU64,
    receipt: CodingApprovalConsumptionReceipt,
) -> Result<(), StoreError> {
    let row = sqlx::query(
        "SELECT request.event_json AS request_json, closed.event_json AS closed_json, reservation.event_json AS reservation_json FROM coding_approval_requests a JOIN coding_events request ON request.job_id=a.job_id AND request.sequence=a.request_sequence JOIN coding_events closed ON closed.job_id=a.job_id AND closed.sequence=a.closed_by_sequence JOIN coding_budget_reservations b ON b.job_id=a.job_id AND b.reservation_sequence=? JOIN coding_events reservation ON reservation.job_id=b.job_id AND reservation.sequence=b.reservation_sequence WHERE a.job_id=? AND a.request_sequence=? AND a.closed_by_sequence=?",
    )
    .bind(sequence(receipt.reservation_sequence)?)
    .bind(job_id.as_str())
    .bind(sequence(request_sequence)?)
    .bind(sequence(receipt.closure_sequence)?)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?
    .ok_or(StoreError::Conflict)?;
    let request_json: String = row
        .try_get("request_json")
        .map_err(|_| StoreError::Invalid)?;
    let closed_json: String = row
        .try_get("closed_json")
        .map_err(|_| StoreError::Invalid)?;
    let reservation_json: String = row
        .try_get("reservation_json")
        .map_err(|_| StoreError::Invalid)?;
    let request = request_from_json(request_json.as_bytes(), job_id, request_sequence)?;
    let closure = closure_from_json(
        closed_json.as_bytes(),
        job_id,
        request_sequence,
        receipt.closure_sequence,
    )?;
    let ApprovalDisposition::Consumed {
        reservation_sequence,
    } = closure.disposition
    else {
        return Err(StoreError::Conflict);
    };
    let reservation = reservation_from_json(
        reservation_json.as_bytes(),
        job_id,
        receipt.reservation_sequence,
    )?;
    if reservation_sequence != receipt.reservation_sequence
        || reservation.agent_id() != request.agent_id()
        || reservation.invocation_id() != request.invocation_id()
        || closure.agent_id.as_ref() != request.agent_id()
        || closure.invocation_id.as_ref() != request.invocation_id()
    {
        return Err(StoreError::Conflict);
    }
    Ok(())
}

async fn finish(
    transaction: Transaction<'_, Sqlite>,
    outcome: Result<CodingApprovalConsumptionAppend, StoreError>,
) -> Result<CodingApprovalConsumptionAppend, StoreError> {
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
