use std::num::NonZeroU64;

use saya_types::{BudgetReservationV1, BudgetSettlementV1, CodingEventKind, CodingEventV1, JobId};
use sqlx::Row;

use super::events::event_error;
use crate::{SqliteStateStore, StoreError};

const MAX_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingBudgetReservationRecord {
    pub reservation: BudgetReservationV1,
    pub settlement: Option<BudgetSettlementV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingBudgetReservationPage {
    pub items: Vec<CodingBudgetReservationRecord>,
    pub next_cursor: Option<NonZeroU64>,
}

impl SqliteStateStore {
    pub async fn get_coding_budget_reservation(
        &self,
        job_id: &JobId,
        reservation_sequence: NonZeroU64,
    ) -> Result<Option<CodingBudgetReservationRecord>, StoreError> {
        let row = sqlx::query(SELECT_RECORD)
            .bind(job_id.as_str())
            .bind(sequence(reservation_sequence)?)
            .fetch_optional(self.pool().await?)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        row.as_ref().map(record).transpose()
    }

    pub async fn list_coding_budget_reservations(
        &self,
        job_id: &JobId,
        after: Option<NonZeroU64>,
        limit: usize,
    ) -> Result<CodingBudgetReservationPage, StoreError> {
        if limit == 0 {
            return Err(StoreError::Invalid);
        }
        if limit > MAX_PAGE_SIZE {
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
            items
                .last()
                .map(|item| item.reservation.reservation_sequence())
        } else {
            None
        };
        Ok(CodingBudgetReservationPage { items, next_cursor })
    }
}

const SELECT_RECORD: &str = "SELECT r.job_id, r.reservation_sequence, r.settled_by_sequence, reserve.event_json AS reservation_json, settled.event_json AS settlement_json FROM coding_budget_reservations r LEFT JOIN coding_events reserve ON reserve.job_id=r.job_id AND reserve.sequence=r.reservation_sequence LEFT JOIN coding_events settled ON settled.job_id=r.job_id AND settled.sequence=r.settled_by_sequence WHERE r.job_id=? AND r.reservation_sequence=?";
const LIST_RECORDS: &str = "SELECT r.job_id, r.reservation_sequence, r.settled_by_sequence, reserve.event_json AS reservation_json, settled.event_json AS settlement_json FROM coding_budget_reservations r LEFT JOIN coding_events reserve ON reserve.job_id=r.job_id AND reserve.sequence=r.reservation_sequence LEFT JOIN coding_events settled ON settled.job_id=r.job_id AND settled.sequence=r.settled_by_sequence WHERE r.job_id=? AND (? IS NULL OR r.reservation_sequence > ?) ORDER BY r.reservation_sequence LIMIT ?";

pub(super) fn record(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<CodingBudgetReservationRecord, StoreError> {
    let job = JobId::parse(
        &row.try_get::<String, _>("job_id")
            .map_err(|_| StoreError::Invalid)?,
    )
    .map_err(|_| StoreError::Invalid)?;
    let sequence = positive(
        row.try_get("reservation_sequence")
            .map_err(|_| StoreError::Invalid)?,
    )?;
    let reservation_json: Option<String> = row
        .try_get("reservation_json")
        .map_err(|_| StoreError::Invalid)?;
    let reservation = reservation_from_json(
        reservation_json.ok_or(StoreError::Invalid)?.as_bytes(),
        &job,
        sequence,
    )?;
    let settled_by: Option<i64> = row
        .try_get("settled_by_sequence")
        .map_err(|_| StoreError::Invalid)?;
    let settlement_json: Option<String> = row
        .try_get("settlement_json")
        .map_err(|_| StoreError::Invalid)?;
    let settlement = match (settled_by, settlement_json) {
        (None, None) => None,
        (Some(settlement_sequence), Some(json)) => Some(settlement_from_json(
            json.as_bytes(),
            &job,
            sequence,
            positive(settlement_sequence)?,
        )?),
        _ => return Err(StoreError::Invalid),
    };
    if settlement
        .as_ref()
        .is_some_and(|settlement| !matching_keys(&reservation, settlement))
    {
        return Err(StoreError::Invalid);
    }
    Ok(CodingBudgetReservationRecord {
        reservation,
        settlement,
    })
}

pub(crate) fn matching_keys(
    reservation: &BudgetReservationV1,
    settlement: &BudgetSettlementV1,
) -> bool {
    reservation.job_id() == settlement.job_id()
        && reservation.charges().len() == settlement.observations().len()
        && reservation.charges().iter().all(|charge| {
            settlement.observations().iter().any(|observation| {
                charge.dimension() == observation.dimension()
                    && charge.endpoint() == observation.endpoint()
                    && charge.price_metadata_sha256() == observation.price_metadata_sha256()
            })
        })
}

pub(crate) fn reservation_from_json(
    json: &[u8],
    job: &JobId,
    sequence: NonZeroU64,
) -> Result<BudgetReservationV1, StoreError> {
    let event = CodingEventV1::from_json_bytes(json).map_err(event_error)?;
    let CodingEventKind::BudgetReserved { reservation } = event.event() else {
        return Err(StoreError::Invalid);
    };
    if event.job_id() != job
        || event.sequence() != sequence
        || reservation.job_id() != job
        || reservation.reservation_sequence() != sequence
    {
        return Err(StoreError::Invalid);
    }
    Ok((**reservation).clone())
}

fn settlement_from_json(
    json: &[u8],
    job: &JobId,
    reservation_sequence: NonZeroU64,
    settlement_sequence: NonZeroU64,
) -> Result<BudgetSettlementV1, StoreError> {
    let event = CodingEventV1::from_json_bytes(json).map_err(event_error)?;
    let CodingEventKind::BudgetSettled { settlement } = event.event() else {
        return Err(StoreError::Invalid);
    };
    if event.job_id() != job
        || event.sequence() != settlement_sequence
        || settlement.job_id() != job
        || settlement.reservation_sequence() != reservation_sequence
        || settlement.settlement_sequence() != settlement_sequence
    {
        return Err(StoreError::Invalid);
    }
    Ok((**settlement).clone())
}

fn positive(value: i64) -> Result<NonZeroU64, StoreError> {
    u64::try_from(value)
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or(StoreError::Invalid)
}

fn sequence(value: NonZeroU64) -> Result<i64, StoreError> {
    i64::try_from(value.get()).map_err(|_| StoreError::LimitExceeded)
}
