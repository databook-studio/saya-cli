use crate::StoreError;
use sqlx::{Sqlite, SqlitePool, pool::PoolConnection};
use std::time::Duration;

const LOCK_RETRIES: usize = 100;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);

pub(crate) async fn migrate(pool: &SqlitePool) -> Result<(), StoreError> {
    let mut connection = pool.acquire().await.map_err(|_| StoreError::Unavailable)?;
    retry_statement(&mut connection, "BEGIN IMMEDIATE").await?;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let did_work = match version {
        0 => {
            step1(&mut connection).await?;
            step2(&mut connection).await?;
            step3(&mut connection).await?;
            true
        }
        1 => {
            step2(&mut connection).await?;
            step3(&mut connection).await?;
            true
        }
        2 => {
            step3(&mut connection).await?;
            true
        }
        3 => false,
        _ => {
            sqlx::query("ROLLBACK").execute(&mut *connection).await.ok();
            return Err(StoreError::VersionUnsupported);
        }
    };
    sqlx::query("COMMIT")
        .execute(&mut *connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    if did_work {
        retry_statement(&mut connection, "PRAGMA journal_mode = WAL").await?;
    }
    Ok(())
}

async fn step1(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS schema_cache(profile_id TEXT PRIMARY KEY, schema_json TEXT NOT NULL, updated_unix_ms INTEGER NOT NULL, version INTEGER NOT NULL)").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE TABLE IF NOT EXISTS audit_log(id INTEGER PRIMARY KEY, created_unix_ms INTEGER NOT NULL, session_id TEXT, profile_id TEXT NOT NULL, operation TEXT NOT NULL, status TEXT NOT NULL, duration_ms INTEGER NOT NULL, row_count INTEGER, truncated INTEGER)").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 1")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

async fn step2(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS contract_objects(id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, catalog_name TEXT NOT NULL, schema_name TEXT NOT NULL, object_name TEXT NOT NULL, object_kind TEXT NOT NULL, schema_fingerprint TEXT NOT NULL, fingerprint_version INTEGER NOT NULL, first_seen_unix_ms INTEGER NOT NULL, last_seen_unix_ms INTEGER NOT NULL, UNIQUE(profile_id, catalog_name, schema_name, object_name, object_kind))").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS contract_objects_profile ON contract_objects(profile_id)",
    )
    .execute(&mut **connection)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE TABLE IF NOT EXISTS contract_claims(id TEXT PRIMARY KEY, object_id TEXT NOT NULL REFERENCES contract_objects(id), claim_kind TEXT NOT NULL, payload_json TEXT NOT NULL, payload_version INTEGER NOT NULL, origin TEXT NOT NULL, status TEXT NOT NULL, schema_fingerprint TEXT NOT NULL, referenced_columns_json TEXT NOT NULL, created_unix_ms INTEGER NOT NULL, updated_unix_ms INTEGER NOT NULL, last_verified_unix_ms INTEGER, deduplication_key TEXT NOT NULL, UNIQUE(object_id, deduplication_key))").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE INDEX IF NOT EXISTS contract_claims_object_status ON contract_claims(object_id, status)").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE TABLE IF NOT EXISTS contract_evidence(id INTEGER PRIMARY KEY, claim_id TEXT NOT NULL REFERENCES contract_claims(id), evidence_kind TEXT NOT NULL, session_id TEXT, turn_ordinal INTEGER, observed_unix_ms INTEGER NOT NULL)").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    // IFNULL wraps the nullable columns so SQLite treats NULLs as equal in the unique index;
    // without this, the same evidence could be recorded repeatedly and inflate a claim's support.
    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS contract_evidence_unique ON contract_evidence(claim_id, evidence_kind, IFNULL(session_id, ''), IFNULL(turn_ordinal, -1))").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS contract_evidence_claim ON contract_evidence(claim_id)",
    )
    .execute(&mut **connection)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    // contract_events.claim_id has no foreign key by design: events are an append-only audit
    // trail and must outlive a future hard purge of the claim they describe.
    sqlx::query("CREATE TABLE IF NOT EXISTS contract_events(id INTEGER PRIMARY KEY, claim_id TEXT NOT NULL, object_id TEXT NOT NULL, event TEXT NOT NULL, from_status TEXT, to_status TEXT, origin TEXT NOT NULL, created_unix_ms INTEGER NOT NULL, reason TEXT)").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE INDEX IF NOT EXISTS contract_events_claim ON contract_events(claim_id)")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 2")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

/// Step 3 (Phase 5c-1): the preferences table. This is a *new* step, not an
/// amendment to step 2 — `user_version = 2` now carries claims a developer may
/// have stored, so an installed database upgrades in place rather than being
/// rewritten under. The table holds settings, not claims: one value per kind
/// per scope, no lifecycle, no evidence, no audit trail.
async fn step3(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS user_preferences(scope_key TEXT NOT NULL, preference_kind TEXT NOT NULL, value_json TEXT NOT NULL, updated_unix_ms INTEGER NOT NULL, PRIMARY KEY (scope_key, preference_kind))").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 3")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

async fn retry_statement(
    connection: &mut PoolConnection<Sqlite>,
    statement: &str,
) -> Result<(), StoreError> {
    for _ in 0..LOCK_RETRIES {
        if sqlx::query(statement)
            .execute(&mut **connection)
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(LOCK_RETRY_DELAY).await;
    }
    Err(StoreError::Unavailable)
}
