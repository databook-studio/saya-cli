use super::{CodingEventAppend, budget_records::reservation_from_json, events::append_event_in_tx};
use crate::{SqliteStateStore, StoreError};
use saya_types::{
    BudgetDimension, BudgetReservationV1, CodingEventKind, CodingEventV1, TransportSpanV1,
};

impl SqliteStateStore {
    pub async fn record_coding_transport(
        &self,
        event: CodingEventV1,
    ) -> Result<CodingEventAppend, StoreError> {
        let CodingEventKind::TransportObserved { span } = event.event() else {
            return Err(StoreError::Invalid);
        };
        let span = span.clone();
        let data = span.data();
        let mut tx = self
            .pool()
            .await?
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let mut reservations = Vec::with_capacity(2);
        for sequence in [data.reservation_sequence, data.parent_reservation_sequence] {
            let json: Option<String> = sqlx::query_scalar(
                "SELECT event_json FROM coding_events WHERE job_id=? AND sequence=?",
            )
            .bind(data.job_id.as_str())
            .bind(i64::try_from(sequence.get()).map_err(|_| StoreError::LimitExceeded)?)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
            reservations.push(reservation_from_json(
                json.ok_or(StoreError::NotFound)?.as_bytes(),
                &data.job_id,
                sequence,
            )?);
        }
        validate_reservations(&span, &reservations[0], &reservations[1])?;
        let sequence = event.sequence();
        let append = append_event_in_tx(&mut tx, event).await?;
        if matches!(append, CodingEventAppend::Appended(_)) {
            sqlx::query("INSERT INTO coding_transport_observations(job_id,reservation_sequence,observation_sequence) VALUES (?,?,?)")
                .bind(data.job_id.as_str()).bind(data.reservation_sequence.get() as i64).bind(sequence.get() as i64)
                .execute(&mut *tx).await.map_err(|_|StoreError::Conflict)?;
        } else {
            let indexed:Option<i64>=sqlx::query_scalar("SELECT observation_sequence FROM coding_transport_observations WHERE job_id=? AND reservation_sequence=?")
                .bind(data.job_id.as_str()).bind(data.reservation_sequence.get() as i64).fetch_optional(&mut *tx).await.map_err(|_|StoreError::Unavailable)?;
            if indexed != Some(sequence.get() as i64) {
                return Err(StoreError::Invalid);
            }
        }
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        Ok(append)
    }
}
pub(super) fn validate_reservations(
    span: &TransportSpanV1,
    physical: &BudgetReservationV1,
    parent: &BudgetReservationV1,
) -> Result<(), StoreError> {
    let data = span.data();
    if physical.job_id() != &data.job_id
        || parent.job_id() != &data.job_id
        || physical.reservation_sequence() != data.reservation_sequence
        || parent.reservation_sequence() != data.parent_reservation_sequence
        || physical.agent_id() != data.agent_id.as_ref()
        || physical.invocation_id() != data.invocation_id.as_ref()
        || parent.agent_id() != physical.agent_id()
        || parent.invocation_id() != physical.invocation_id()
        || !matches!(physical.charges().len(), 1 | 3)
        || physical.charges()[0].dimension() != BudgetDimension::TransportAttempts
        || !parent
            .charges()
            .iter()
            .any(|charge| charge.dimension() == BudgetDimension::LogicalRequests)
    {
        return Err(StoreError::Invalid);
    }
    Ok(())
}
