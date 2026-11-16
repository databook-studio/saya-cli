use std::num::NonZeroU64;

use saya_types::{CodingEventV1, JobId};
use sqlx::Row;

use super::{MAX_CODING_EVENT_PAGE_SIZE, events::event_error};
use crate::{SqliteStateStore, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingEventPage {
    pub events: Vec<CodingEventV1>,
    pub next_cursor: Option<NonZeroU64>,
}

impl SqliteStateStore {
    /// An unknown job has no events and returns an empty page.
    pub async fn list_coding_events(
        &self,
        job_id: &JobId,
        after_sequence: Option<NonZeroU64>,
        limit: usize,
    ) -> Result<CodingEventPage, StoreError> {
        if limit == 0 {
            return Err(StoreError::Invalid);
        }
        if limit > MAX_CODING_EVENT_PAGE_SIZE {
            return Err(StoreError::LimitExceeded);
        }
        let after = after_sequence
            .map(|sequence| i64::try_from(sequence.get()).map_err(|_| StoreError::LimitExceeded))
            .transpose()?
            .unwrap_or(0);
        let rows = sqlx::query("SELECT job_id, sequence, event_json FROM coding_events WHERE job_id=? AND sequence>? ORDER BY sequence LIMIT ?")
            .bind(job_id.as_str())
            .bind(after)
            .bind(i64::try_from(limit + 1).map_err(|_| StoreError::LimitExceeded)?)
            .fetch_all(self.pool().await?)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let mut events: Vec<_> = rows
            .iter()
            .map(|row| decode(row, job_id))
            .collect::<Result<_, _>>()?;
        let next_cursor = if events.len() > limit {
            events.pop();
            events.last().map(|event| event.sequence())
        } else {
            None
        };
        Ok(CodingEventPage {
            events,
            next_cursor,
        })
    }
}

fn decode(
    row: &sqlx::sqlite::SqliteRow,
    requested_job: &JobId,
) -> Result<CodingEventV1, StoreError> {
    let indexed_job = JobId::parse(
        &row.try_get::<String, _>("job_id")
            .map_err(|_| StoreError::Invalid)?,
    )
    .map_err(|_| StoreError::Invalid)?;
    let indexed_sequence = row
        .try_get::<i64, _>("sequence")
        .map_err(|_| StoreError::Invalid)?;
    let indexed_sequence = u64::try_from(indexed_sequence)
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or(StoreError::Invalid)?;
    let event_json: String = row.try_get("event_json").map_err(|_| StoreError::Invalid)?;
    let event = CodingEventV1::from_json_bytes(event_json.as_bytes()).map_err(event_error)?;
    if &indexed_job != requested_job
        || event.job_id() != &indexed_job
        || event.sequence() != indexed_sequence
    {
        return Err(StoreError::Invalid);
    }
    Ok(event)
}
