use crate::StoreError;
use sqlx::{Sqlite, pool::PoolConnection};

pub(crate) async fn step10(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS coding_jobs(job_id TEXT PRIMARY KEY, repository_id TEXT NOT NULL, snapshot_id TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('queued', 'running', 'waiting_for_approval', 'paused', 'completed', 'failed', 'cancelled', 'uncertain')), created_unix_ms INTEGER NOT NULL CHECK(created_unix_ms >= 0), updated_unix_ms INTEGER NOT NULL CHECK(updated_unix_ms >= created_unix_ms))")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE TABLE IF NOT EXISTS coding_children(job_id TEXT NOT NULL REFERENCES coding_jobs(job_id) ON DELETE RESTRICT, agent_id TEXT NOT NULL, current_invocation_id TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('queued', 'running', 'waiting_for_approval', 'paused', 'completed', 'failed', 'cancelled', 'uncertain')), created_unix_ms INTEGER NOT NULL CHECK(created_unix_ms >= 0), updated_unix_ms INTEGER NOT NULL CHECK(updated_unix_ms >= created_unix_ms), PRIMARY KEY(job_id, agent_id), UNIQUE(job_id, current_invocation_id))")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 10")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

pub(crate) async fn step11(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("ALTER TABLE coding_children ADD COLUMN result_source_sha256 TEXT")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("ALTER TABLE coding_children ADD COLUMN result_outcome TEXT CHECK((result_source_sha256 IS NULL) = (result_outcome IS NULL) AND (result_outcome IS NULL OR result_outcome IN ('completed', 'failed', 'cancelled', 'uncertain')))")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE TABLE coding_events(job_id TEXT NOT NULL REFERENCES coding_jobs(job_id) ON DELETE RESTRICT, sequence INTEGER NOT NULL CHECK(sequence > 0), event_json TEXT NOT NULL CHECK(length(CAST(event_json AS BLOB)) <= 65536), PRIMARY KEY(job_id, sequence))")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 11")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}
