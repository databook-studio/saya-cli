use crate::{SqliteStateStore, StoreError};
use saya_types::{
    BudgetEffectState, BudgetObservationValue, BudgetReservationV1, BudgetSettlementV1,
    CodingEventKind, CodingEventV1, JobId, TransportOutcome, TransportSpanV1,
};
use sqlx::Row;
use std::num::NonZeroU64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingTransportAttemptRecord {
    pub reservation: BudgetReservationV1,
    pub span: Option<TransportSpanV1>,
    pub settlement: Option<BudgetSettlementV1>,
}
impl CodingTransportAttemptRecord {
    /// Complete client timing is distinct from known remote outcome/usage.
    pub fn timing_complete(&self) -> bool {
        self.span.is_some() && self.settlement.is_some()
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingTransportAttemptPage {
    pub items: Vec<CodingTransportAttemptRecord>,
    pub next_cursor: Option<NonZeroU64>,
}
impl SqliteStateStore {
    /// Includes missing spans/settlements. Never silently drops interrupted attempts.
    pub async fn list_coding_transport_attempts(
        &self,
        job: &JobId,
        after: Option<NonZeroU64>,
        limit: usize,
    ) -> Result<CodingTransportAttemptPage, StoreError> {
        if limit == 0 {
            return Err(StoreError::Invalid);
        }
        if limit > 100 {
            return Err(StoreError::LimitExceeded);
        }
        let after = after
            .map(|n| i64::try_from(n.get()))
            .transpose()
            .map_err(|_| StoreError::LimitExceeded)?
            .unwrap_or(0);
        let rows = sqlx::query(LIST)
            .bind(job.as_str())
            .bind(after)
            .bind((limit + 1) as i64)
            .fetch_all(self.pool().await?)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let mut items = rows.iter().map(decode).collect::<Result<Vec<_>, _>>()?;
        let next_cursor = if items.len() > limit {
            items.pop();
            items
                .last()
                .map(|item| item.reservation.reservation_sequence())
        } else {
            None
        };
        Ok(CodingTransportAttemptPage { items, next_cursor })
    }
}
const LIST: &str = "SELECT r.job_id,r.reservation_sequence,r.settled_by_sequence,reserve.event_json AS reservation_json,settled.event_json AS settlement_json,t.observation_sequence,observed.event_json AS span_json,parent.event_json AS parent_json FROM coding_budget_reservations r LEFT JOIN coding_events reserve ON reserve.job_id=r.job_id AND reserve.sequence=r.reservation_sequence LEFT JOIN coding_events settled ON settled.job_id=r.job_id AND settled.sequence=r.settled_by_sequence LEFT JOIN coding_transport_observations t ON t.job_id=r.job_id AND t.reservation_sequence=r.reservation_sequence LEFT JOIN coding_events observed ON observed.job_id=t.job_id AND observed.sequence=t.observation_sequence LEFT JOIN coding_events parent ON parent.job_id=r.job_id AND parent.sequence=CAST(json_extract(observed.event_json,'$.event.span.parent_reservation_sequence') AS INTEGER) WHERE r.job_id=? AND r.reservation_sequence>? AND EXISTS(SELECT 1 FROM json_each(json_extract(reserve.event_json,'$.event.reservation.charges')) c WHERE json_extract(c.value,'$.dimension')='transport_attempts') ORDER BY r.reservation_sequence LIMIT ?";
fn decode(row: &sqlx::sqlite::SqliteRow) -> Result<CodingTransportAttemptRecord, StoreError> {
    let budget = super::budget_records::record(row)?;
    let indexed: Option<i64> = row
        .try_get("observation_sequence")
        .map_err(|_| StoreError::Invalid)?;
    let json: Option<String> = row.try_get("span_json").map_err(|_| StoreError::Invalid)?;
    let span = match (indexed, json) {
        (None, None) => None,
        (Some(sequence), Some(json)) => {
            let event =
                CodingEventV1::from_json_bytes(json.as_bytes()).map_err(|_| StoreError::Invalid)?;
            if i64::try_from(event.sequence().get()).ok() != Some(sequence) {
                return Err(StoreError::Invalid);
            }
            let CodingEventKind::TransportObserved { span } = event.event() else {
                return Err(StoreError::Invalid);
            };
            let parent: Option<String> = row
                .try_get("parent_json")
                .map_err(|_| StoreError::Invalid)?;
            let parent = super::budget_records::reservation_from_json(
                parent.ok_or(StoreError::Invalid)?.as_bytes(),
                budget.reservation.job_id(),
                span.data().parent_reservation_sequence,
            )?;
            super::transport::validate_reservations(span, &budget.reservation, &parent)?;
            Some((**span).clone())
        }
        _ => return Err(StoreError::Invalid),
    };
    if let (Some(span), Some(settlement)) = (&span, &budget.settlement) {
        let (state, value) = match span.data().outcome {
            TransportOutcome::NotStarted => (
                BudgetEffectState::Settled,
                BudgetObservationValue::Measured(0),
            ),
            TransportOutcome::Completed => (
                BudgetEffectState::Settled,
                BudgetObservationValue::Measured(1),
            ),
            TransportOutcome::Unknown => (
                BudgetEffectState::Uncertain,
                BudgetObservationValue::Unknown,
            ),
        };
        if (state == BudgetEffectState::Uncertain && settlement.effect_state() != state)
            || settlement
                .observations()
                .iter()
                .find(|observation| {
                    observation.dimension() == saya_types::BudgetDimension::TransportAttempts
                })
                .is_none_or(|observation| observation.value() != value)
        {
            return Err(StoreError::Invalid);
        }
    }
    Ok(CodingTransportAttemptRecord {
        reservation: budget.reservation,
        span,
        settlement: budget.settlement,
    })
}
