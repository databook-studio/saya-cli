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
            step4(&mut connection).await?;
            step5(&mut connection).await?;
            step6(&mut connection).await?;
            true
        }
        1 => {
            step2(&mut connection).await?;
            step3(&mut connection).await?;
            step4(&mut connection).await?;
            step5(&mut connection).await?;
            step6(&mut connection).await?;
            true
        }
        2 => {
            step3(&mut connection).await?;
            step4(&mut connection).await?;
            step5(&mut connection).await?;
            step6(&mut connection).await?;
            true
        }
        3 => {
            step4(&mut connection).await?;
            step5(&mut connection).await?;
            step6(&mut connection).await?;
            true
        }
        4 => {
            step5(&mut connection).await?;
            step6(&mut connection).await?;
            true
        }
        5 => {
            step6(&mut connection).await?;
            true
        }
        6 => false,
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

/// Step 4 (Spec C): the claim's own fingerprint version. A claim was decoded
/// under the *object* row's `fingerprint_version`, which `upsert_object_in_tx`
/// overwrites on every schema refresh — so a claim written under version A read
/// back under whatever version the object row carries now, silently
/// misclassifying it at the first format change. The version now travels with
/// the claim: a `fingerprint_version` column on `contract_claims`, written at
/// propose time from the fingerprint the caller supplied.
///
/// A new step, not an amendment to step 2: `user_version = 3` carries claims a
/// developer's unreleased database may already hold, and step 2's
/// `CREATE TABLE IF NOT EXISTS` is a no-op on an existing table, so it cannot
/// add the column in place. Step 4 owns the column for every database — the
/// `ALTER TABLE` upgrades an installed v3 database without rewriting the rows
/// it exists to preserve, and the explicit presence check makes it idempotent
/// on a fresh database that ran step 2 still missing the column. SQLite's
/// `ALTER TABLE ADD COLUMN` lacks `IF NOT EXISTS`, so the check is manual.
async fn step4(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    let has_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('contract_claims') WHERE name='fingerprint_version'",
    )
    .fetch_one(&mut **connection)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if has_column == 0 {
        sqlx::query(
            "ALTER TABLE contract_claims ADD COLUMN fingerprint_version INTEGER NOT NULL DEFAULT 1",
        )
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    }
    sqlx::query("PRAGMA user_version = 4")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

/// Step 5 (Spec D-3): the `knowledge_items` table — one current-state row per
/// knowledge slot, where object identity is *inlined* rather than joined to
/// `contract_objects`. That join was where the version defect lived: a claim
/// decoded under the object row's `fingerprint_version`, which a refresh
/// overwrites, so the version now travels on the row that carries the binding
/// it describes (`fingerprint_version` here, written from the caller's
/// fingerprint at insert time).
///
/// Cardinality is enforced at the storage boundary, not only in the type: a
/// single-valued slot (`table.grain`, `table.default_time`, `column:<c>.role`)
/// admits one row per object. SQL cannot see the slot's typed cardinality, so
/// the row carries a `cardinality` column the partial unique index below keys
/// on — `WHERE cardinality='single'` over the object identity plus slot. A
/// Rust-side guard alone is a convention; this index is the contract.
///
/// A new step, not an amendment: nothing has shipped, so there is no data to
/// preserve, and `CREATE TABLE IF NOT EXISTS` is idempotent on a fresh database.
/// The existing `contract_*` tables and their callers are untouched.
async fn step5(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS knowledge_items(id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, catalog TEXT NOT NULL, schema TEXT NOT NULL, object TEXT NOT NULL, object_kind TEXT NOT NULL, slot TEXT NOT NULL, cardinality TEXT NOT NULL, value_json TEXT NOT NULL, source TEXT NOT NULL, state TEXT NOT NULL, schema_binding_json TEXT NOT NULL, fingerprint_version INTEGER NOT NULL, created_unix_ms INTEGER NOT NULL, updated_unix_ms INTEGER NOT NULL)").execute(&mut **connection).await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS knowledge_items_profile ON knowledge_items(profile_id)",
    )
    .execute(&mut **connection)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("CREATE INDEX IF NOT EXISTS knowledge_items_object ON knowledge_items(profile_id, catalog, schema, object, object_kind)")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    // The storage-boundary invariant: one row per single-valued slot per object.
    // A partial unique index — multi-valued slots are excluded so their several
    // rows coexist, and `cardinality='multi'` rows never collide here. Writing a
    // second value to a single-valued slot must hit this and force a replace,
    // not a second insert.
    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS knowledge_items_single ON knowledge_items(profile_id, catalog, schema, object, object_kind, slot) WHERE cardinality = 'single'")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 5")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

/// Step 6 (Spec G, Chunk 5): drop the legacy `contract_*` tables. Phases D and
/// F moved every read and write to `knowledge_items`, so the four legacy tables
/// are dead weight. Nothing has shipped, so there is no data to preserve and no
/// compatibility window: drop. `user_preferences`, `schema_cache`, `audit_log`,
/// and `knowledge_items` are untouched (the knowledge path is the product now).
///
/// A new step, not an amendment: `user_version = 5` is a real state a
/// developer's unreleased database may hold, and a `CREATE TABLE IF NOT EXISTS`
/// step cannot remove a table in place. `IF EXISTS` makes the drop safe on every
/// path, including a fresh database that never created the tables.
async fn step6(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    // Drop order: leaves before roots, so a foreign-key-enabled connection never
    // dangles a reference mid-drop. `contract_evidence` and `contract_events`
    // both reference `contract_claims`; `contract_claims` references
    // `contract_objects`.
    sqlx::query("DROP TABLE IF EXISTS contract_evidence")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("DROP TABLE IF EXISTS contract_events")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("DROP TABLE IF EXISTS contract_claims")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("DROP TABLE IF EXISTS contract_objects")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("PRAGMA user_version = 6")
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
