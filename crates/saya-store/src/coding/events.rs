use std::num::NonZeroU64;

use saya_types::{CodingEventError, CodingEventKind, CodingEventV1};
use sqlx::{Sqlite, Transaction};

use super::projection::apply_common_projection;
use crate::{SqliteStateStore, StoreError};

pub const MAX_CODING_EVENT_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodingEventAppend {
    Appended(NonZeroU64),
    AlreadyPresent(NonZeroU64),
}

impl SqliteStateStore {
    pub async fn append_coding_event(
        &self,
        event: CodingEventV1,
    ) -> Result<CodingEventAppend, StoreError> {
        if !matches!(
            event.event(),
            CodingEventKind::StateChanged { .. } | CodingEventKind::ResultReported { .. }
        ) {
            return Err(StoreError::Invalid);
        }
        let pool = self.pool().await?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        match append_event_in_tx(&mut transaction, event).await {
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
}

pub(crate) async fn append_event_in_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    event: CodingEventV1,
) -> Result<CodingEventAppend, StoreError> {
    let sequence = i64::try_from(event.sequence().get()).map_err(|_| StoreError::LimitExceeded)?;
    let event_json = String::from_utf8(event.to_json_bytes().map_err(event_error)?)
        .map_err(|_| StoreError::Invalid)?;
    let existing: Option<String> =
        sqlx::query_scalar("SELECT event_json FROM coding_events WHERE job_id=? AND sequence=?")
            .bind(event.job_id().as_str())
            .bind(sequence)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    if let Some(existing) = existing {
        let stored = CodingEventV1::from_json_bytes(existing.as_bytes()).map_err(event_error)?;
        return if stored == event {
            Ok(CodingEventAppend::AlreadyPresent(event.sequence()))
        } else {
            Err(StoreError::Conflict)
        };
    }
    let last: Option<i64> =
        sqlx::query_scalar("SELECT MAX(sequence) FROM coding_events WHERE job_id=?")
            .bind(event.job_id().as_str())
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    let expected = match last {
        Some(last) => last.checked_add(1).ok_or(StoreError::LimitExceeded)?,
        None => 1,
    };
    if sequence != expected {
        return Err(StoreError::Conflict);
    }
    sqlx::query("INSERT INTO coding_events(job_id, sequence, event_json) VALUES (?, ?, ?)")
        .bind(event.job_id().as_str())
        .bind(sequence)
        .bind(event_json)
        .execute(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    apply_common_projection(transaction, &event).await?;
    Ok(CodingEventAppend::Appended(event.sequence()))
}

pub(crate) fn event_error(error: CodingEventError) -> StoreError {
    match error {
        CodingEventError::UnsupportedVersion(_) => StoreError::VersionUnsupported,
        _ => StoreError::Invalid,
    }
}
