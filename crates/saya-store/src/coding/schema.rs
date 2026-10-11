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

pub(crate) async fn step12(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE coding_budget_reservations(job_id TEXT NOT NULL, reservation_sequence INTEGER NOT NULL CHECK(reservation_sequence > 0), settled_by_sequence INTEGER CHECK(settled_by_sequence > 0), PRIMARY KEY(job_id, reservation_sequence), FOREIGN KEY(job_id, reservation_sequence) REFERENCES coding_events(job_id, sequence) ON DELETE RESTRICT, FOREIGN KEY(job_id, settled_by_sequence) REFERENCES coding_events(job_id, sequence) ON DELETE RESTRICT)")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 12")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

pub(crate) async fn step13(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE coding_managed_checkouts(job_id TEXT NOT NULL, checkout_id TEXT NOT NULL, created_by_sequence INTEGER NOT NULL CHECK(created_by_sequence > 0), latest_operation_sequence INTEGER NOT NULL CHECK(latest_operation_sequence >= created_by_sequence), PRIMARY KEY(job_id, checkout_id), FOREIGN KEY(job_id, created_by_sequence) REFERENCES coding_events(job_id, sequence) ON DELETE RESTRICT, FOREIGN KEY(job_id, latest_operation_sequence) REFERENCES coding_events(job_id, sequence) ON DELETE RESTRICT)")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE TABLE coding_checkout_operations(job_id TEXT NOT NULL, operation_sequence INTEGER NOT NULL CHECK(operation_sequence > 0), checkout_id TEXT NOT NULL, action TEXT NOT NULL CHECK(action IN ('create_checkout', 'remove_checkout', 'integrate_candidate', 'apply_back')), outcome_sequence INTEGER CHECK(outcome_sequence IS NULL OR outcome_sequence > operation_sequence), PRIMARY KEY(job_id, operation_sequence), FOREIGN KEY(job_id, operation_sequence) REFERENCES coding_events(job_id, sequence) ON DELETE RESTRICT, FOREIGN KEY(job_id, checkout_id) REFERENCES coding_managed_checkouts(job_id, checkout_id) ON DELETE RESTRICT, FOREIGN KEY(job_id, outcome_sequence) REFERENCES coding_events(job_id, sequence) ON DELETE RESTRICT)")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE UNIQUE INDEX coding_checkout_one_open_operation ON coding_checkout_operations(job_id, checkout_id) WHERE outcome_sequence IS NULL")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE UNIQUE INDEX coding_checkout_one_outcome_event ON coding_checkout_operations(job_id, outcome_sequence) WHERE outcome_sequence IS NOT NULL")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 13")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

pub(crate) async fn step14(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE coding_approval_requests(job_id TEXT NOT NULL, request_sequence INTEGER NOT NULL CHECK(request_sequence > 0), closed_by_sequence INTEGER CHECK(closed_by_sequence > 0), PRIMARY KEY(job_id, request_sequence), FOREIGN KEY(job_id, request_sequence) REFERENCES coding_events(job_id, sequence) ON DELETE RESTRICT, FOREIGN KEY(job_id, closed_by_sequence) REFERENCES coding_events(job_id, sequence) ON DELETE RESTRICT)")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 14")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

pub(crate) async fn step15(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE coding_budget_policies(job_id TEXT PRIMARY KEY REFERENCES coding_jobs(job_id) ON DELETE RESTRICT, policy_sha256 TEXT NOT NULL, policy_json TEXT NOT NULL CHECK(length(CAST(policy_json AS BLOB)) <= 65536))")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 15")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

pub(crate) async fn step16(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE coding_checkout_plans(job_id TEXT NOT NULL, managed_checkout_id TEXT NOT NULL, plan_sha256 TEXT NOT NULL, plan_bytes BLOB NOT NULL, PRIMARY KEY(job_id, managed_checkout_id), FOREIGN KEY(job_id, managed_checkout_id) REFERENCES coding_managed_checkouts(job_id, checkout_id) ON DELETE RESTRICT)")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 16")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

pub(crate) async fn step17(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE coding_completions(job_id TEXT NOT NULL, sequence INTEGER NOT NULL CHECK(sequence>0), agent_id TEXT NOT NULL, invocation_id TEXT NOT NULL, result_bytes BLOB NOT NULL CHECK(length(result_bytes)<=65536), acknowledged INTEGER NOT NULL CHECK(acknowledged IN (0,1)), PRIMARY KEY(job_id, sequence), UNIQUE(job_id, agent_id, invocation_id), FOREIGN KEY(job_id, sequence) REFERENCES coding_events(job_id, sequence) ON DELETE RESTRICT, FOREIGN KEY(job_id, agent_id) REFERENCES coding_children(job_id, agent_id) ON DELETE RESTRICT)")
        .execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE TABLE coding_runtime_authority(job_id TEXT PRIMARY KEY REFERENCES coding_jobs(job_id) ON DELETE RESTRICT, epoch INTEGER NOT NULL CHECK(epoch>0), policy_digest TEXT NOT NULL)").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE TABLE coding_cancel_requests(job_id TEXT PRIMARY KEY REFERENCES coding_jobs(job_id) ON DELETE RESTRICT)").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 17")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

pub(crate) async fn step18(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE coding_transport_observations(job_id TEXT NOT NULL,reservation_sequence INTEGER NOT NULL,observation_sequence INTEGER NOT NULL,PRIMARY KEY(job_id,reservation_sequence),UNIQUE(job_id,observation_sequence),FOREIGN KEY(job_id,reservation_sequence) REFERENCES coding_budget_reservations(job_id,reservation_sequence) ON DELETE RESTRICT,FOREIGN KEY(job_id,observation_sequence) REFERENCES coding_events(job_id,sequence) ON DELETE RESTRICT)")
        .execute(&mut **connection).await.map_err(|_|StoreError::Unavailable)?;
    sqlx::query("CREATE TABLE coding_token_policies(job_id TEXT PRIMARY KEY REFERENCES coding_jobs(job_id) ON DELETE RESTRICT,policy_json BLOB NOT NULL CHECK(length(policy_json)<=8192),policy_sha256 TEXT NOT NULL)").execute(&mut **connection).await.map_err(|_|StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 18")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}
