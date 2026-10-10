use saya_types::{
    AgentId, CodingResultOutcome, InvocationId, JobId, JobState, RepositoryId, Sha256Digest,
    SnapshotId,
};
use sqlx::Row;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{SqliteStateStore, StoreError};

const MAX_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCodingJob {
    pub job_id: JobId,
    pub repository_id: RepositoryId,
    pub snapshot_id: SnapshotId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingJobRecord {
    pub job_id: JobId,
    pub repository_id: RepositoryId,
    pub snapshot_id: SnapshotId,
    pub state: JobState,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCodingChild {
    pub job_id: JobId,
    pub agent_id: AgentId,
    pub current_invocation_id: InvocationId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingChildRecord {
    pub job_id: JobId,
    pub agent_id: AgentId,
    pub current_invocation_id: InvocationId,
    pub state: JobState,
    pub result_source_sha256: Option<Sha256Digest>,
    pub result_outcome: Option<CodingResultOutcome>,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingChildPage {
    pub items: Vec<CodingChildRecord>,
    pub next_cursor: Option<AgentId>,
}

impl SqliteStateStore {
    pub async fn create_coding_job(
        &self,
        job: NewCodingJob,
    ) -> Result<CodingJobRecord, StoreError> {
        let stamp = now()?;
        let inserted = sqlx::query("INSERT INTO coding_jobs(job_id, repository_id, snapshot_id, state, created_unix_ms, updated_unix_ms) VALUES (?, ?, ?, 'queued', ?, ?) ON CONFLICT DO NOTHING")
            .bind(job.job_id.as_str())
            .bind(job.repository_id.as_str())
            .bind(job.snapshot_id.as_str())
            .bind(stamp)
            .bind(stamp)
            .execute(self.pool().await?)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        if inserted.rows_affected() == 0 {
            return Err(StoreError::Conflict);
        }
        Ok(CodingJobRecord {
            job_id: job.job_id,
            repository_id: job.repository_id,
            snapshot_id: job.snapshot_id,
            state: JobState::Queued,
            created_unix_ms: stamp,
            updated_unix_ms: stamp,
        })
    }

    pub async fn get_coding_job(
        &self,
        job_id: &JobId,
    ) -> Result<Option<CodingJobRecord>, StoreError> {
        let row = sqlx::query("SELECT job_id, repository_id, snapshot_id, state, created_unix_ms, updated_unix_ms FROM coding_jobs WHERE job_id=?")
            .bind(job_id.as_str())
            .fetch_optional(self.pool().await?)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        row.as_ref().map(job_record).transpose()
    }

    pub async fn create_coding_child(
        &self,
        child: NewCodingChild,
    ) -> Result<CodingChildRecord, StoreError> {
        if self.get_coding_job(&child.job_id).await?.is_none() {
            return Err(StoreError::NotFound);
        }
        let stamp = now()?;
        let inserted = sqlx::query("INSERT INTO coding_children(job_id, agent_id, current_invocation_id, state, created_unix_ms, updated_unix_ms) VALUES (?, ?, ?, 'queued', ?, ?) ON CONFLICT DO NOTHING")
            .bind(child.job_id.as_str())
            .bind(child.agent_id.as_str())
            .bind(child.current_invocation_id.as_str())
            .bind(stamp)
            .bind(stamp)
            .execute(self.pool().await?)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        if inserted.rows_affected() == 0 {
            return Err(StoreError::Conflict);
        }
        Ok(CodingChildRecord {
            job_id: child.job_id,
            agent_id: child.agent_id,
            current_invocation_id: child.current_invocation_id,
            state: JobState::Queued,
            result_source_sha256: None,
            result_outcome: None,
            created_unix_ms: stamp,
            updated_unix_ms: stamp,
        })
    }

    /// An unknown job has no children and returns an empty page.
    pub async fn list_coding_children(
        &self,
        job_id: &JobId,
        after_agent_id: Option<&AgentId>,
        limit: usize,
    ) -> Result<CodingChildPage, StoreError> {
        if limit == 0 {
            return Err(StoreError::Invalid);
        }
        if limit > MAX_PAGE_SIZE {
            return Err(StoreError::LimitExceeded);
        }
        let rows = sqlx::query("SELECT job_id, agent_id, current_invocation_id, state, result_source_sha256, result_outcome, created_unix_ms, updated_unix_ms FROM coding_children WHERE job_id=? AND (? IS NULL OR agent_id > ?) ORDER BY agent_id LIMIT ?")
            .bind(job_id.as_str())
            .bind(after_agent_id.map(AgentId::as_str))
            .bind(after_agent_id.map(AgentId::as_str))
            .bind(i64::try_from(limit + 1).map_err(|_| StoreError::LimitExceeded)?)
            .fetch_all(self.pool().await?)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let mut items: Vec<_> = rows.iter().map(child_record).collect::<Result<_, _>>()?;
        let next_cursor = if items.len() > limit {
            items
                .pop()
                .and_then(|_| items.last().map(|item| item.agent_id.clone()))
        } else {
            None
        };
        Ok(CodingChildPage { items, next_cursor })
    }
}

pub(crate) fn now() -> Result<i64, StoreError> {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StoreError::Invalid)?
        .as_millis();
    i64::try_from(milliseconds).map_err(|_| StoreError::LimitExceeded)
}

fn job_record(row: &sqlx::sqlite::SqliteRow) -> Result<CodingJobRecord, StoreError> {
    Ok(CodingJobRecord {
        job_id: JobId::parse(&text(row, "job_id")?).map_err(|_| StoreError::Invalid)?,
        repository_id: RepositoryId::parse(&text(row, "repository_id")?)
            .map_err(|_| StoreError::Invalid)?,
        snapshot_id: SnapshotId::parse(&text(row, "snapshot_id")?)
            .map_err(|_| StoreError::Invalid)?,
        state: state(&text(row, "state")?)?,
        created_unix_ms: timestamp(row, "created_unix_ms")?,
        updated_unix_ms: timestamp_pair(row)?,
    })
}

fn child_record(row: &sqlx::sqlite::SqliteRow) -> Result<CodingChildRecord, StoreError> {
    let (result_source_sha256, result_outcome) = result_projection(row)?;
    Ok(CodingChildRecord {
        job_id: JobId::parse(&text(row, "job_id")?).map_err(|_| StoreError::Invalid)?,
        agent_id: AgentId::parse(&text(row, "agent_id")?).map_err(|_| StoreError::Invalid)?,
        current_invocation_id: InvocationId::parse(&text(row, "current_invocation_id")?)
            .map_err(|_| StoreError::Invalid)?,
        state: state(&text(row, "state")?)?,
        result_source_sha256,
        result_outcome,
        created_unix_ms: timestamp(row, "created_unix_ms")?,
        updated_unix_ms: timestamp_pair(row)?,
    })
}

fn text(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<String, StoreError> {
    row.try_get(column).map_err(|_| StoreError::Invalid)
}

fn timestamp(row: &sqlx::sqlite::SqliteRow, column: &str) -> Result<i64, StoreError> {
    let value = row.try_get(column).map_err(|_| StoreError::Invalid)?;
    (value >= 0).then_some(value).ok_or(StoreError::Invalid)
}

fn timestamp_pair(row: &sqlx::sqlite::SqliteRow) -> Result<i64, StoreError> {
    let created = timestamp(row, "created_unix_ms")?;
    let updated = timestamp(row, "updated_unix_ms")?;
    (updated >= created)
        .then_some(updated)
        .ok_or(StoreError::Invalid)
}

pub(crate) fn state(value: &str) -> Result<JobState, StoreError> {
    match value {
        "queued" => Ok(JobState::Queued),
        "running" => Ok(JobState::Running),
        "waiting_for_approval" => Ok(JobState::WaitingForApproval),
        "paused" => Ok(JobState::Paused),
        "completed" => Ok(JobState::Completed),
        "failed" => Ok(JobState::Failed),
        "cancelled" => Ok(JobState::Cancelled),
        "uncertain" => Ok(JobState::Uncertain),
        _ => Err(StoreError::Invalid),
    }
}

fn result_projection(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<(Option<Sha256Digest>, Option<CodingResultOutcome>), StoreError> {
    let source = row
        .try_get::<Option<String>, _>("result_source_sha256")
        .map_err(|_| StoreError::Invalid)?;
    let outcome = row
        .try_get::<Option<String>, _>("result_outcome")
        .map_err(|_| StoreError::Invalid)?;
    match (source, outcome) {
        (None, None) => Ok((None, None)),
        (Some(source), Some(outcome)) => Ok((
            Some(Sha256Digest::parse(&source).map_err(|_| StoreError::Invalid)?),
            Some(match outcome.as_str() {
                "completed" => CodingResultOutcome::Completed,
                "failed" => CodingResultOutcome::Failed,
                "cancelled" => CodingResultOutcome::Cancelled,
                "uncertain" => CodingResultOutcome::Uncertain,
                _ => return Err(StoreError::Invalid),
            }),
        )),
        _ => Err(StoreError::Invalid),
    }
}
