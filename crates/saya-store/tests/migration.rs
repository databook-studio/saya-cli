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
const CONTRACT_TABLES: [&str; 4] = [
    "contract_objects",
    "contract_claims",
    "contract_evidence",
    "contract_events",
];
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

#[tokio::test]
async fn fresh_database_reaches_version_two() {
    let root = temp_root("fresh");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(user_version(&db).await, 2);
    let tables = contract_tables(&db).await;
    for expected in CONTRACT_TABLES {
        assert!(
            tables.iter().any(|name| name == expected),
            "missing {expected}"
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn upgrade_from_version_one_preserves_data() {
    let root = temp_root("upgrade");
    let db = root.join("state.sqlite3");
    build_version_one_database(&db).await;
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(user_version(&db).await, 2);
    let tables = contract_tables(&db).await;
    for expected in CONTRACT_TABLES {
        assert!(
            tables.iter().any(|name| name == expected),
            "missing {expected}"
        );
    }
    let pool = read_pool(&db).await;
    let (json, updated, version): (String, i64, i64) = sqlx::query_as(
        "SELECT schema_json, updated_unix_ms, version FROM schema_cache WHERE profile_id='p-test'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(json, "{}");
    assert_eq!(updated, 11111);
    assert_eq!(version, 1);
    let row: AuditRow =
        sqlx::query_as("SELECT created_unix_ms, session_id, profile_id, operation, status, duration_ms, row_count, truncated FROM audit_log ORDER BY id LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.0, 22222);
    assert!(row.1.is_none());
    assert_eq!(row.2, "p-test");
    assert_eq!(row.3, "query");
    assert_eq!(row.4, "success");
    assert_eq!(row.5, 10);
    assert_eq!(row.6, Some(5));
    assert_eq!(row.7, Some(0));
    pool.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn unknown_future_version_fails_closed() {
    let root = temp_root("future");
    let db = root.join("state.sqlite3");
    let pool = create_pool(&db).await;
    sqlx::query("PRAGMA user_version = 3")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let before = fs::read(&db).unwrap();
    let store = SqliteStateStore::new(&db);
    let error = store.list_schema_metadata().await.unwrap_err();
    assert_eq!(error, StoreError::VersionUnsupported);
    let after = fs::read(&db).unwrap();
    assert_eq!(before, after);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn migration_is_idempotent() {
    let root = temp_root("idempotent");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(user_version(&db).await, 2);
    let reopened = SqliteStateStore::new(&db);
    reopened.list_schema_metadata().await.unwrap();
    reopened.close().await;
    assert_eq!(user_version(&db).await, 2);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn wal_is_on_after_upgrade() {
    let root = temp_root("wal");
    let db = root.join("state.sqlite3");
    build_version_one_database(&db).await;
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(journal_mode(&db).await, "wal");
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[tokio::test]
async fn permissions_survive_after_upgrade() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("perms");
    let db = root.join("state.sqlite3");
    build_version_one_database(&db).await;
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(
        fs::metadata(&db).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let _ = fs::remove_dir_all(root);
}

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

// --- Review addition: the tests above prove the contract tables EXIST, which would
// --- still pass if a UNIQUE constraint were missing. Deduplication, contradiction
// --- detection, and evidence bounding are all enforced by those constraints rather
// --- than by application code, so their absence must fail here and not silently in
// --- a later phase.

async fn contract_pool(db: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db).foreign_keys(true))
        .await
        .unwrap()
}

async fn migrated_database(label: &str) -> (PathBuf, PathBuf) {
    let root = temp_root(label);
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    (root, db)
}

const INSERT_OBJECT: &str = "INSERT INTO contract_objects(id, profile_id, catalog_name, schema_name, object_name, object_kind, schema_fingerprint, fingerprint_version, first_seen_unix_ms, last_seen_unix_ms) VALUES (?, 'p-a', 'cat', 'sch', 'orders', 'table', 'ff', 1, 1, 1)";
const INSERT_CLAIM: &str = "INSERT INTO contract_claims(id, object_id, claim_kind, payload_json, payload_version, origin, status, schema_fingerprint, referenced_columns_json, created_unix_ms, updated_unix_ms, deduplication_key) VALUES (?, 'o-1', 'table_alias', '{}', 1, 'user_explicit', 'confirmed', 'ff', '[]', 1, 1, ?)";

#[tokio::test]
async fn one_object_identity_cannot_be_stored_twice() {
    let (root, db) = migrated_database("uniq-object").await;
    let pool = contract_pool(&db).await;
    sqlx::query(INSERT_OBJECT)
        .bind("o-1")
        .execute(&pool)
        .await
        .unwrap();
    // Same qualified identity under a different surrogate id must still collide.
    let second = sqlx::query(INSERT_OBJECT).bind("o-2").execute(&pool).await;
    assert!(
        second.is_err(),
        "contract_objects is missing its identity UNIQUE constraint"
    );
    pool.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn one_deduplication_key_cannot_be_stored_twice_per_object() {
    let (root, db) = migrated_database("uniq-claim").await;
    let pool = contract_pool(&db).await;
    sqlx::query(INSERT_OBJECT)
        .bind("o-1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(INSERT_CLAIM)
        .bind("c-1")
        .bind("d-same")
        .execute(&pool)
        .await
        .unwrap();
    let duplicate = sqlx::query(INSERT_CLAIM)
        .bind("c-2")
        .bind("d-same")
        .execute(&pool)
        .await;
    assert!(
        duplicate.is_err(),
        "contract_claims is missing its deduplication UNIQUE constraint"
    );
    // A different key on the same object is a distinct claim, not a collision.
    sqlx::query(INSERT_CLAIM)
        .bind("c-3")
        .bind("d-other")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn repeated_evidence_with_null_columns_cannot_inflate_a_claim() {
    let (root, db) = migrated_database("uniq-evidence").await;
    let pool = contract_pool(&db).await;
    sqlx::query(INSERT_OBJECT)
        .bind("o-1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(INSERT_CLAIM)
        .bind("c-1")
        .bind("d-1")
        .execute(&pool)
        .await
        .unwrap();

    let insert = "INSERT INTO contract_evidence(claim_id, evidence_kind, session_id, turn_ordinal, observed_unix_ms) VALUES ('c-1', 'explicit_user_statement', ?, ?, 1)";
    let null: Option<String> = None;
    let no_turn: Option<i64> = None;
    sqlx::query(insert)
        .bind(&null)
        .bind(no_turn)
        .execute(&pool)
        .await
        .unwrap();

    // SQLite treats NULLs as distinct in a UNIQUE index, so without the IFNULL
    // wrapper this identical row would insert again and inflate apparent support.
    let repeat = sqlx::query(insert)
        .bind(&null)
        .bind(no_turn)
        .execute(&pool)
        .await;
    assert!(
        repeat.is_err(),
        "contract_evidence unique index does not collapse NULL columns"
    );

    // Evidence differing only in turn ordinal is genuinely new.
    sqlx::query(insert)
        .bind(&null)
        .bind(Some(2_i64))
        .execute(&pool)
        .await
        .unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM contract_evidence WHERE claim_id='c-1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 2);
    pool.close().await;
    let _ = fs::remove_dir_all(root);
}
