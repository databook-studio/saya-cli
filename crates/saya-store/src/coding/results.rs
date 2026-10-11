//! Atomic joined output and its stable completion notification identity.
use super::{CodingEventAppend, events::append_event_in_tx};
use crate::{SqliteStateStore, StoreError};
use saya_types::{CodingEventKind, CodingEventV1, JoinedResultV1};
use sqlx::Row;

pub const MAX_COMPLETIONS_PER_JOB: i64 = 1_024;

impl SqliteStateStore {
    /// Atomically appends the terminal event and complete bounded result. Repeating
    /// the exact same event/result returns its original notification identity.
    pub async fn record_coding_completion(
        &self,
        event: CodingEventV1,
        result: &JoinedResultV1,
    ) -> Result<CodingEventAppend, StoreError> {
        validate(&event, result)?;
        let bytes = result.to_json_bytes().map_err(|_| StoreError::Invalid)?;
        let sequence =
            i64::try_from(event.sequence().get()).map_err(|_| StoreError::LimitExceeded)?;
        let mut tx = self
            .pool()
            .await?
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let existing = sqlx::query("SELECT sequence, result_bytes FROM coding_completions WHERE job_id=? AND agent_id=? AND invocation_id=?")
            .bind(result.job_id().as_str()).bind(result.agent_id().as_str()).bind(result.invocation_id().as_str())
            .fetch_optional(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
        if let Some(row) = existing {
            let stored_sequence: i64 = row.try_get("sequence").map_err(|_| StoreError::Invalid)?;
            let stored_bytes: Vec<u8> = row
                .try_get("result_bytes")
                .map_err(|_| StoreError::Invalid)?;
            if stored_sequence != sequence || stored_bytes != bytes {
                return Err(StoreError::Conflict);
            }
            // Also verify the durable event: a corrupt projection cannot certify replay.
            let append = append_event_in_tx(&mut tx, event).await?;
            if !matches!(append, CodingEventAppend::AlreadyPresent(_)) {
                return Err(StoreError::Invalid);
            }
            tx.commit().await.map_err(|_| StoreError::Unavailable)?;
            return Ok(append);
        }
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM coding_completions WHERE job_id=?")
                .bind(result.job_id().as_str())
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| StoreError::Unavailable)?;
        if count >= MAX_COMPLETIONS_PER_JOB {
            return Err(StoreError::LimitExceeded);
        }
        let append = append_event_in_tx(&mut tx, event).await?;
        if !matches!(append, CodingEventAppend::Appended(_)) {
            return Err(StoreError::Conflict);
        }
        sqlx::query("INSERT INTO coding_completions(job_id, sequence, agent_id, invocation_id, result_bytes, acknowledged) VALUES (?, ?, ?, ?, ?, 0)")
            .bind(result.job_id().as_str()).bind(sequence).bind(result.agent_id().as_str())
            .bind(result.invocation_id().as_str()).bind(bytes).execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        Ok(append)
    }
}

pub(super) fn validate(event: &CodingEventV1, result: &JoinedResultV1) -> Result<(), StoreError> {
    if event.job_id() != result.job_id()
        || event.agent_id() != Some(result.agent_id())
        || event.invocation_id() != Some(result.invocation_id())
        || !matches!(event.event(), CodingEventKind::StateChanged { state } if *state == result.terminal_state())
    {
        return Err(StoreError::Invalid);
    }
    Ok(())
}
