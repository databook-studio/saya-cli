use super::CodingJobRecord;
use crate::{SqliteStateStore, StoreError};
use saya_types::JobId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingJobPage {
    pub items: Vec<CodingJobRecord>,
    pub next_cursor: Option<JobId>,
}
impl SqliteStateStore {
    pub async fn list_coding_jobs(
        &self,
        after: Option<&JobId>,
        limit: usize,
    ) -> Result<CodingJobPage, StoreError> {
        if limit == 0 {
            return Err(StoreError::Invalid);
        }
        if limit > 100 {
            return Err(StoreError::LimitExceeded);
        }
        let rows = sqlx::query("SELECT job_id, repository_id, snapshot_id, state, created_unix_ms, updated_unix_ms FROM coding_jobs WHERE (? IS NULL OR job_id>?) ORDER BY job_id LIMIT ?")
            .bind(after.map(JobId::as_str)).bind(after.map(JobId::as_str)).bind((limit+1) as i64)
            .fetch_all(self.pool().await?).await.map_err(|_| StoreError::Unavailable)?;
        let mut items = rows
            .iter()
            .map(super::records::job_record)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = if items.len() > limit {
            items.pop();
            items.last().map(|item| item.job_id.clone())
        } else {
            None
        };
        Ok(CodingJobPage { items, next_cursor })
    }
}
