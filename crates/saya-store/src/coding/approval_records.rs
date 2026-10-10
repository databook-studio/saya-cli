use std::num::NonZeroU64;

use saya_types::{
    AgentId, ApprovalDisposition, ApprovalRequestV1, CodingEventKind, CodingEventV1, InvocationId,
    JobId,
};
use sqlx::{Row, Sqlite, Transaction};

use super::budget_records::reservation_from_json;
use super::events::event_error;
use crate::{SqliteStateStore, StoreError};

pub(crate) const MAX_APPROVAL_REQUEST_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingApprovalClosure {
    pub closure_sequence: NonZeroU64,
    pub agent_id: Option<AgentId>,
    pub invocation_id: Option<InvocationId>,
    pub disposition: ApprovalDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingApprovalRequestRecord {
    pub request: ApprovalRequestV1,
    pub closure: Option<CodingApprovalClosure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingApprovalRequestPage {
    pub items: Vec<CodingApprovalRequestRecord>,
    pub next_cursor: Option<NonZeroU64>,
}

impl SqliteStateStore {
    pub async fn get_coding_approval_request(
        &self,
        job_id: &JobId,
        request_sequence: NonZeroU64,
    ) -> Result<Option<CodingApprovalRequestRecord>, StoreError> {
        let row = sqlx::query(SELECT_RECORD)
            .bind(job_id.as_str())
            .bind(sequence(request_sequence)?)
            .fetch_optional(self.pool().await?)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        row.as_ref().map(record).transpose()
    }

    pub async fn list_coding_approval_requests(
        &self,
        job_id: &JobId,
        after: Option<NonZeroU64>,
        limit: usize,
    ) -> Result<CodingApprovalRequestPage, StoreError> {
        if limit == 0 {
            return Err(StoreError::Invalid);
        }
        if limit > MAX_APPROVAL_REQUEST_PAGE_SIZE {
            return Err(StoreError::LimitExceeded);
        }
        let cursor = after.map(sequence).transpose()?;
        let rows = sqlx::query(LIST_RECORDS)
            .bind(job_id.as_str())
            .bind(cursor)
            .bind(cursor)
            .bind(i64::try_from(limit + 1).map_err(|_| StoreError::LimitExceeded)?)
            .fetch_all(self.pool().await?)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let mut items = rows.iter().map(record).collect::<Result<Vec<_>, _>>()?;
        let next_cursor = if items.len() > limit {
            items.pop();
            items.last().map(|item| item.request.request_sequence())
        } else {
            None
        };
        Ok(CodingApprovalRequestPage { items, next_cursor })
    }
}

const SELECT_RECORD: &str = "SELECT r.job_id, r.request_sequence, r.closed_by_sequence, request.event_json AS request_json, closed.event_json AS closed_json, reservation.event_json AS reservation_json FROM coding_approval_requests r LEFT JOIN coding_events request ON request.job_id=r.job_id AND request.sequence=r.request_sequence LEFT JOIN coding_events closed ON closed.job_id=r.job_id AND closed.sequence=r.closed_by_sequence LEFT JOIN coding_budget_reservations reserved ON reserved.job_id=r.job_id AND reserved.reservation_sequence=CAST(json_extract(closed.event_json, '$.event.disposition.reservation_sequence') AS INTEGER) LEFT JOIN coding_events reservation ON reservation.job_id=reserved.job_id AND reservation.sequence=reserved.reservation_sequence WHERE r.job_id=? AND r.request_sequence=?";
const LIST_RECORDS: &str = "SELECT r.job_id, r.request_sequence, r.closed_by_sequence, request.event_json AS request_json, closed.event_json AS closed_json, reservation.event_json AS reservation_json FROM coding_approval_requests r LEFT JOIN coding_events request ON request.job_id=r.job_id AND request.sequence=r.request_sequence LEFT JOIN coding_events closed ON closed.job_id=r.job_id AND closed.sequence=r.closed_by_sequence LEFT JOIN coding_budget_reservations reserved ON reserved.job_id=r.job_id AND reserved.reservation_sequence=CAST(json_extract(closed.event_json, '$.event.disposition.reservation_sequence') AS INTEGER) LEFT JOIN coding_events reservation ON reservation.job_id=reserved.job_id AND reservation.sequence=reserved.reservation_sequence WHERE r.job_id=? AND (? IS NULL OR r.request_sequence > ?) ORDER BY r.request_sequence LIMIT ?";

pub(crate) async fn pending_request(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &JobId,
    request_sequence: NonZeroU64,
) -> Result<ApprovalRequestV1, StoreError> {
    let row = sqlx::query(
        "SELECT r.closed_by_sequence, e.event_json FROM coding_approval_requests r LEFT JOIN coding_events e ON e.job_id=r.job_id AND e.sequence=r.request_sequence WHERE r.job_id=? AND r.request_sequence=?",
    )
    .bind(job_id.as_str())
    .bind(sequence(request_sequence)?)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?
    .ok_or(StoreError::NotFound)?;
    let closed: Option<i64> = row
        .try_get("closed_by_sequence")
        .map_err(|_| StoreError::Invalid)?;
    if closed.is_some() {
        return Err(StoreError::Conflict);
    }
    let json: Option<String> = row.try_get("event_json").map_err(|_| StoreError::Invalid)?;
    request_from_json(
        json.ok_or(StoreError::Invalid)?.as_bytes(),
        job_id,
        request_sequence,
    )
}

fn record(row: &sqlx::sqlite::SqliteRow) -> Result<CodingApprovalRequestRecord, StoreError> {
    let job = JobId::parse(&text(row, "job_id")?).map_err(|_| StoreError::Invalid)?;
    let request_sequence = positive(
        row.try_get("request_sequence")
            .map_err(|_| StoreError::Invalid)?,
    )?;
    let request = request_from_json(
        text(row, "request_json")?.as_bytes(),
        &job,
        request_sequence,
    )?;
    let closure_sequence: Option<i64> = row
        .try_get("closed_by_sequence")
        .map_err(|_| StoreError::Invalid)?;
    let closure_json: Option<String> = row
        .try_get("closed_json")
        .map_err(|_| StoreError::Invalid)?;
    let closure = match (closure_sequence, closure_json) {
        (None, None) => None,
        (Some(sequence), Some(json)) => Some(closure_from_json(
            json.as_bytes(),
            &job,
            request_sequence,
            positive(sequence)?,
        )?),
        _ => return Err(StoreError::Invalid),
    };
    if closure.as_ref().is_some_and(|closure| {
        closure.agent_id.as_ref() != request.agent_id()
            || closure.invocation_id.as_ref() != request.invocation_id()
    }) {
        return Err(StoreError::Invalid);
    }
    if let Some(CodingApprovalClosure {
        closure_sequence,
        disposition: ApprovalDisposition::Consumed {
            reservation_sequence,
        },
        ..
    }) = closure.as_ref()
    {
        if reservation_sequence.get().checked_add(1) != Some(closure_sequence.get()) {
            return Err(StoreError::Invalid);
        }
        let reservation_json: Option<String> = row
            .try_get("reservation_json")
            .map_err(|_| StoreError::Invalid)?;
        let reservation = reservation_from_json(
            reservation_json.ok_or(StoreError::Invalid)?.as_bytes(),
            &job,
            *reservation_sequence,
        )?;
        if reservation.agent_id() != request.agent_id()
            || reservation.invocation_id() != request.invocation_id()
        {
            return Err(StoreError::Invalid);
        }
    }
    Ok(CodingApprovalRequestRecord { request, closure })
}

pub(crate) fn request_from_json(
    json: &[u8],
    job_id: &JobId,
    request_sequence: NonZeroU64,
) -> Result<ApprovalRequestV1, StoreError> {
    let event = CodingEventV1::from_json_bytes(json).map_err(event_error)?;
    let CodingEventKind::ApprovalRequested { request } = event.event() else {
        return Err(StoreError::Invalid);
    };
    if event.job_id() != job_id
        || event.sequence() != request_sequence
        || request.job_id() != job_id
        || request.request_sequence() != request_sequence
        || event.agent_id() != request.agent_id()
        || event.invocation_id() != request.invocation_id()
    {
        return Err(StoreError::Invalid);
    }
    Ok((**request).clone())
}

pub(crate) fn closure_from_json(
    json: &[u8],
    job_id: &JobId,
    request_sequence: NonZeroU64,
    closure_sequence: NonZeroU64,
) -> Result<CodingApprovalClosure, StoreError> {
    let event = CodingEventV1::from_json_bytes(json).map_err(event_error)?;
    let CodingEventKind::ApprovalClosed {
        request_sequence: referenced_request,
        disposition,
    } = event.event()
    else {
        return Err(StoreError::Invalid);
    };
    if event.job_id() != job_id
        || event.sequence() != closure_sequence
        || *referenced_request != request_sequence
        || event.agent_id().is_some() != event.invocation_id().is_some()
    {
        return Err(StoreError::Invalid);
    }
    Ok(CodingApprovalClosure {
        closure_sequence,
        agent_id: event.agent_id().cloned(),
        invocation_id: event.invocation_id().cloned(),
        disposition: disposition.clone(),
    })
}

pub(crate) fn sequence(value: NonZeroU64) -> Result<i64, StoreError> {
    i64::try_from(value.get()).map_err(|_| StoreError::LimitExceeded)
}

fn positive(value: i64) -> Result<NonZeroU64, StoreError> {
    u64::try_from(value)
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or(StoreError::Invalid)
}

fn text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, StoreError> {
    row.try_get(column).map_err(|_| StoreError::Invalid)
}
