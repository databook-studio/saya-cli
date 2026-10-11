use crate::{SqliteStateStore, StoreError};
use saya_types::{CodingEventV1, JobId, JoinedResultV1};
use sqlx::Row;
use std::num::NonZeroU64;

pub const MAX_COMPLETION_PAGE_SIZE: usize = 32;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingCompletion {
    /// Stable notification identity together with result.job_id().
    pub sequence: NonZeroU64,
    pub result: JoinedResultV1,
    pub acknowledged: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingCompletionPage {
    pub items: Vec<CodingCompletion>,
    pub next_cursor: Option<NonZeroU64>,
}
impl SqliteStateStore {
    /// Retrieves the durable accepted result too, so ACK never deletes its content.
    pub async fn get_coding_completion(
        &self,
        job: &JobId,
        sequence: NonZeroU64,
    ) -> Result<Option<CodingCompletion>, StoreError> {
        let sequence = i64::try_from(sequence.get()).map_err(|_| StoreError::LimitExceeded)?;
        let row = sqlx::query("SELECT c.acknowledged, c.sequence, c.agent_id, c.invocation_id, c.result_bytes, e.event_json FROM coding_completions c JOIN coding_events e ON e.job_id=c.job_id AND e.sequence=c.sequence WHERE c.job_id=? AND c.sequence=?")
            .bind(job.as_str()).bind(sequence).fetch_optional(self.pool().await?).await.map_err(|_| StoreError::Unavailable)?;
        row.as_ref().map(|row| decode(row, job)).transpose()
    }

    /// At-least-once delivery with a stable identity; only an explicit ACK removes
    /// a result from replay. Reading never marks it delivered or executes effects.
    pub async fn list_coding_completions(
        &self,
        job: &JobId,
        after: Option<NonZeroU64>,
        limit: usize,
    ) -> Result<CodingCompletionPage, StoreError> {
        self.completions_page(job, after, limit, false).await
    }

    /// Bounded continuation/history read including already accepted results.
    pub async fn list_coding_completion_history(
        &self,
        job: &JobId,
        after: Option<NonZeroU64>,
        limit: usize,
    ) -> Result<CodingCompletionPage, StoreError> {
        self.completions_page(job, after, limit, true).await
    }

    async fn completions_page(
        &self,
        job: &JobId,
        after: Option<NonZeroU64>,
        limit: usize,
        include_accepted: bool,
    ) -> Result<CodingCompletionPage, StoreError> {
        if limit == 0 {
            return Err(StoreError::Invalid);
        }
        if limit > MAX_COMPLETION_PAGE_SIZE {
            return Err(StoreError::LimitExceeded);
        }
        let after = after
            .map(|n| i64::try_from(n.get()))
            .transpose()
            .map_err(|_| StoreError::LimitExceeded)?
            .unwrap_or(0);
        let rows = sqlx::query("SELECT c.acknowledged, c.sequence, c.agent_id, c.invocation_id, c.result_bytes, e.event_json FROM coding_completions c JOIN coding_events e ON e.job_id=c.job_id AND e.sequence=c.sequence WHERE c.job_id=? AND c.sequence>? AND (? OR c.acknowledged=0) ORDER BY c.sequence LIMIT ?")
            .bind(job.as_str()).bind(after).bind(include_accepted).bind((limit + 1) as i64).fetch_all(self.pool().await?).await.map_err(|_| StoreError::Unavailable)?;
        let mut items = rows
            .iter()
            .map(|row| decode(row, job))
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = if items.len() > limit {
            items.pop();
            items.last().map(|item| item.sequence)
        } else {
            None
        };
        Ok(CodingCompletionPage { items, next_cursor })
    }
    /// Returns true only on the first durable acceptance; false on duplicate ACK.
    pub async fn ack_coding_completion(
        &self,
        job: &JobId,
        sequence: NonZeroU64,
    ) -> Result<bool, StoreError> {
        let sequence = i64::try_from(sequence.get()).map_err(|_| StoreError::LimitExceeded)?;
        let mut tx = self
            .pool()
            .await?
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let current: Option<i64> = sqlx::query_scalar(
            "SELECT acknowledged FROM coding_completions WHERE job_id=? AND sequence=?",
        )
        .bind(job.as_str())
        .bind(sequence)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        let current = current.ok_or(StoreError::NotFound)?;
        if current == 0 {
            sqlx::query(
                "UPDATE coding_completions SET acknowledged=1 WHERE job_id=? AND sequence=?",
            )
            .bind(job.as_str())
            .bind(sequence)
            .execute(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        } else if current != 1 {
            return Err(StoreError::Invalid);
        }
        tx.commit().await.map_err(|_| StoreError::Unavailable)?;
        Ok(current == 0)
    }
}
fn decode(row: &sqlx::sqlite::SqliteRow, job: &JobId) -> Result<CodingCompletion, StoreError> {
    let sequence: i64 = row.try_get("sequence").map_err(|_| StoreError::Invalid)?;
    let sequence = u64::try_from(sequence)
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or(StoreError::Invalid)?;
    let bytes: Vec<u8> = row
        .try_get("result_bytes")
        .map_err(|_| StoreError::Invalid)?;
    let result = JoinedResultV1::from_json_bytes(&bytes).map_err(|_| StoreError::Invalid)?;
    let event: String = row.try_get("event_json").map_err(|_| StoreError::Invalid)?;
    let event =
        CodingEventV1::from_json_bytes(event.as_bytes()).map_err(|_| StoreError::Invalid)?;
    let agent: String = row.try_get("agent_id").map_err(|_| StoreError::Invalid)?;
    let invocation: String = row
        .try_get("invocation_id")
        .map_err(|_| StoreError::Invalid)?;
    if result.job_id() != job
        || result.agent_id().as_str() != agent
        || result.invocation_id().as_str() != invocation
        || event.sequence() != sequence
    {
        return Err(StoreError::Invalid);
    }
    super::results::validate(&event, &result)?;
    let acknowledged: i64 = row
        .try_get("acknowledged")
        .map_err(|_| StoreError::Invalid)?;
    if !matches!(acknowledged, 0 | 1) {
        return Err(StoreError::Invalid);
    }
    Ok(CodingCompletion {
        sequence,
        result,
        acknowledged: acknowledged == 1,
    })
}
