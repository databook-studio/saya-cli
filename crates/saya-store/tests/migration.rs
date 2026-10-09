use saya_store::{SchemaStore, SqliteStateStore, StoreError};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const STEP1_SCHEMA_CACHE: &str = "CREATE TABLE IF NOT EXISTS schema_cache(profile_id TEXT PRIMARY KEY, schema_json TEXT NOT NULL, updated_unix_ms INTEGER NOT NULL, version INTEGER NOT NULL)";
const STEP1_AUDIT_LOG: &str = "CREATE TABLE IF NOT EXISTS audit_log(id INTEGER PRIMARY KEY, created_unix_ms INTEGER NOT NULL, session_id TEXT, profile_id TEXT NOT NULL, operation TEXT NOT NULL, status TEXT NOT NULL, duration_ms INTEGER NOT NULL, row_count INTEGER, truncated INTEGER)";
type AuditRow = (
    i64,
    Option<String>,
    String,
    String,
    String,
    i64,
    Option<i64>,
    Option<i64>,
);

#[path = "migration/current_schema.rs"]
mod current_schema;
#[path = "migration/safety.rs"]
mod safety;
#[path = "migration/upgrade_paths.rs"]
mod upgrade_paths;

async fn create_pool(db: &Path) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(db)
        .create_if_missing(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap()
}

async fn read_pool(db: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap()
}

async fn build_version_one_database(db: &Path) {
    let pool = create_pool(db).await;
    sqlx::query(STEP1_SCHEMA_CACHE)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(STEP1_AUDIT_LOG).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO schema_cache(profile_id, schema_json, updated_unix_ms, version) VALUES ('p-test', '{}', 11111, 1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO audit_log(created_unix_ms, session_id, profile_id, operation, status, duration_ms, row_count, truncated) VALUES (22222, NULL, 'p-test', 'query', 'success', 10, 5, 0)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version = 1")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

// Step 2's contract schema, mirrored verbatim so a v2 database built here is
// byte-identical to one the real migration would have produced.
const STEP2_OBJECTS: &str = "CREATE TABLE contract_objects(id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, catalog_name TEXT NOT NULL, schema_name TEXT NOT NULL, object_name TEXT NOT NULL, object_kind TEXT NOT NULL, schema_fingerprint TEXT NOT NULL, fingerprint_version INTEGER NOT NULL, first_seen_unix_ms INTEGER NOT NULL, last_seen_unix_ms INTEGER NOT NULL, UNIQUE(profile_id, catalog_name, schema_name, object_name, object_kind))";
const STEP2_CLAIMS: &str = "CREATE TABLE contract_claims(id TEXT PRIMARY KEY, object_id TEXT NOT NULL REFERENCES contract_objects(id), claim_kind TEXT NOT NULL, payload_json TEXT NOT NULL, payload_version INTEGER NOT NULL, origin TEXT NOT NULL, status TEXT NOT NULL, schema_fingerprint TEXT NOT NULL, referenced_columns_json TEXT NOT NULL, created_unix_ms INTEGER NOT NULL, updated_unix_ms INTEGER NOT NULL, last_verified_unix_ms INTEGER, deduplication_key TEXT NOT NULL, UNIQUE(object_id, deduplication_key))";
const STEP2_EVIDENCE: &str = "CREATE TABLE contract_evidence(id INTEGER PRIMARY KEY, claim_id TEXT NOT NULL REFERENCES contract_claims(id), evidence_kind TEXT NOT NULL, session_id TEXT, turn_ordinal INTEGER, observed_unix_ms INTEGER NOT NULL)";
const STEP2_EVENTS: &str = "CREATE TABLE contract_events(id INTEGER PRIMARY KEY, claim_id TEXT NOT NULL, object_id TEXT NOT NULL, event TEXT NOT NULL, from_status TEXT, to_status TEXT, origin TEXT NOT NULL, created_unix_ms INTEGER NOT NULL, reason TEXT)";

/// Builds a database at `user_version = 2` with one object, one claim, two
/// evidence rows, and one event — the four row kinds a developer's installed
/// database may already hold. Step 3 must upgrade it without losing any.
async fn build_version_two_database(db: &Path) {
    let pool = create_pool(db).await;
    sqlx::query(STEP1_SCHEMA_CACHE)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(STEP1_AUDIT_LOG).execute(&pool).await.unwrap();
    sqlx::query(STEP2_OBJECTS).execute(&pool).await.unwrap();
    sqlx::query(STEP2_CLAIMS).execute(&pool).await.unwrap();
    sqlx::query(STEP2_EVIDENCE).execute(&pool).await.unwrap();
    sqlx::query(STEP2_EVENTS).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO contract_objects(id, profile_id, catalog_name, schema_name, object_name, object_kind, schema_fingerprint, fingerprint_version, first_seen_unix_ms, last_seen_unix_ms) VALUES ('o-survives', 'p-test', 'cat', 'sch', 'orders', 'table', 'ff', 1, 11111, 22222)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO contract_claims(id, object_id, claim_kind, payload_json, payload_version, origin, status, schema_fingerprint, referenced_columns_json, created_unix_ms, updated_unix_ms, last_verified_unix_ms, deduplication_key) VALUES ('c-survives', 'o-survives', 'table_alias', '{\"kind\":\"table_alias\",\"alias\":\"o\"}', 2, 'user_explicit', 'confirmed', 'ff', '[]', 11111, 22222, NULL, 'd-survives')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO contract_evidence(claim_id, evidence_kind, session_id, turn_ordinal, observed_unix_ms) VALUES ('c-survives', 'explicit_user_statement', 's1', 1, 33333)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO contract_evidence(claim_id, evidence_kind, session_id, turn_ordinal, observed_unix_ms) VALUES ('c-survives', 'successful_read_query', 's1', 2, 44444)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO contract_events(claim_id, object_id, event, from_status, to_status, origin, created_unix_ms, reason) VALUES ('c-survives', 'o-survives', 'proposed', NULL, 'confirmed', 'user_explicit', 11111, NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version = 2")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

/// A database at `user_version = 4` — the latest before step 5. Built from the
/// v2 schema plus step 4's `ALTER TABLE` (the column the v4 ladder added),
/// backfilled on the existing claim, so it is byte-identical to what a v4 build
/// would have produced before step 5 existed.
async fn build_version_four_database(db: &Path) {
    build_version_two_database(db).await;
    let pool = create_pool(db).await;
    sqlx::query(
        "ALTER TABLE contract_claims ADD COLUMN fingerprint_version INTEGER NOT NULL DEFAULT 1",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE contract_claims SET fingerprint_version=1 WHERE id='c-survives'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version = 4")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

async fn user_version(db: &Path) -> i64 {
    let pool = read_pool(db).await;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    version
}

async fn contract_tables(db: &Path) -> Vec<String> {
    let pool = read_pool(db).await;
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'contract_%'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    pool.close().await;
    rows
}

/// True if `table` exists in `sqlite_master`. `user_preferences` is not a
/// `contract_%` table, so it needs its own existence check.
async fn table_exists(db: &Path, table: &str) -> bool {
    let pool = read_pool(db).await;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?")
            .bind(table)
            .fetch_one(&pool)
            .await
            .unwrap();
    pool.close().await;
    count > 0
}

async fn column_exists(db: &Path, table: &str, column: &str) -> bool {
    let pool = read_pool(db).await;
    let sql = format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name=?");
    let count = sqlx::query_scalar::<_, i64>(&sql)
        .bind(column)
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    count > 0
}

async fn journal_mode(db: &Path) -> String {
    let pool = read_pool(db).await;
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    mode
}

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-migration-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}
