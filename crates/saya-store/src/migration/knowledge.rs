use crate::StoreError;
use sqlx::{Sqlite, pool::PoolConnection};

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
pub(super) async fn step4(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
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
pub(super) async fn step5(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
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

/// Step 8 records whether a forgotten row's post-commit byte cleanup finished.
/// The default keeps existing tombstones compatible: their payload was already
/// checkpointed by the old writer, so they need no recovery pass.
pub(super) async fn step8(connection: &mut PoolConnection<Sqlite>) -> Result<(), StoreError> {
    let has_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('knowledge_items') WHERE name='cleanup_state'",
    )
    .fetch_one(&mut **connection)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if has_column == 0 {
        sqlx::query(
            "ALTER TABLE knowledge_items ADD COLUMN cleanup_state TEXT NOT NULL DEFAULT 'complete'",
        )
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    }
    sqlx::query("PRAGMA user_version = 8")
        .execute(&mut **connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}
