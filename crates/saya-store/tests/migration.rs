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
async fn fresh_database_reaches_version_five() {
    let root = temp_root("fresh");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(user_version(&db).await, 5);
    let tables = contract_tables(&db).await;
    for expected in CONTRACT_TABLES {
        assert!(
            tables.iter().any(|name| name == expected),
            "missing {expected}"
        );
    }
    assert!(
        table_exists(&db, "user_preferences").await,
        "missing user_preferences"
    );
    // Step 5 adds the knowledge_items table; a fresh database reaches it.
    assert!(
        table_exists(&db, "knowledge_items").await,
        "missing knowledge_items"
    );
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
    assert_eq!(user_version(&db).await, 5);
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

/// A `user_version = 2` database — one a developer may already have, since the
/// migration is unreleased — upgrades to 3 and keeps every existing claim,
/// object, evidence and event row (spec test 7). Step 3 adds a *new* table; it
/// must not touch the rows the ladder exists to preserve. Asserts the counts of
/// all four contract tables and the full contents of one object row.
#[tokio::test]
async fn upgrade_from_version_two_preserves_every_contract_row() {
    let root = temp_root("upgrade-v2");
    let db = root.join("state.sqlite3");
    build_version_two_database(&db).await;
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(user_version(&db).await, 5);
    assert!(
        table_exists(&db, "user_preferences").await,
        "upgrade did not add user_preferences"
    );
    // Step 5 adds knowledge_items; an upgraded v2 database reaches it too.
    assert!(
        table_exists(&db, "knowledge_items").await,
        "upgrade did not add knowledge_items"
    );
    // Step 4 added the claim's own fingerprint-version column to the legacy
    // v2 table (step 2's CREATE is a no-op on an existing table, so the ALTER
    // is what upgrades an installed database). A claim written under version A
    // must decode under A even after the object row drifts — see contract_store.
    assert!(
        claim_has_fingerprint_version_column(&db).await,
        "upgrade did not add contract_claims.fingerprint_version"
    );

    let pool = read_pool(&db).await;
    // One object, one claim, two evidence rows, one event — all survive.
    let objects: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contract_objects")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(objects, 1);
    let claims: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contract_claims")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(claims, 1);
    let evidence: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contract_evidence")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(evidence, 2);
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contract_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(events, 1);

    // The one object row survives byte-for-byte: identity, kind, fingerprint.
    let row: (String, String, String, String, String, String, String, i64, i64, i64) =
        sqlx::query_as("SELECT id, profile_id, catalog_name, schema_name, object_name, object_kind, schema_fingerprint, fingerprint_version, first_seen_unix_ms, last_seen_unix_ms FROM contract_objects ORDER BY id LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let (id, profile_id, catalog, schema, object, kind, fp, fpv, first, last) = row;
    assert_eq!(id, "o-survives");
    assert_eq!(profile_id, "p-test");
    assert_eq!(catalog, "cat");
    assert_eq!(schema, "sch");
    assert_eq!(object, "orders");
    assert_eq!(kind, "table");
    assert_eq!(fp, "ff");
    assert_eq!(fpv, 1);
    assert_eq!(first, 11111);
    assert_eq!(last, 22222);
    pool.close().await;
    let _ = fs::remove_dir_all(root);
}

/// A `user_version = 4` database — the latest before step 5 — upgrades to 5 and
/// gains `knowledge_items` without losing the contract row it already holds.
/// Step 5 adds a new table; like step 3, it must not touch the rows the ladder
/// exists to preserve.
#[tokio::test]
async fn upgrade_from_version_four_adds_knowledge_items() {
    let root = temp_root("upgrade-v4");
    let db = root.join("state.sqlite3");
    build_version_four_database(&db).await;
    let store = SqliteStateStore::new(&db);
    store.list_schema_metadata().await.unwrap();
    store.close().await;
    assert_eq!(user_version(&db).await, 5);
    assert!(
        table_exists(&db, "knowledge_items").await,
        "upgrade did not add knowledge_items"
    );
    // The claim written under v4 survives the v5 upgrade untouched.
    let pool = read_pool(&db).await;
    let claims: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contract_claims")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(claims, 1);
    pool.close().await;
    let _ = fs::remove_dir_all(root);
}

/// A version ahead of the highest step the migration knows about fails closed.
/// Step 5 makes `user_version = 5` supported, so the future-version sentinel is
/// now 6 — anything the running build cannot migrate *to* must be refused, not
/// silently rewritten under.
#[tokio::test]
async fn unknown_future_version_fails_closed() {
    let root = temp_root("future");
    let db = root.join("state.sqlite3");
    let pool = create_pool(&db).await;
    sqlx::query("PRAGMA user_version = 6")
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
    assert_eq!(user_version(&db).await, 5);
    let reopened = SqliteStateStore::new(&db);
    reopened.list_schema_metadata().await.unwrap();
    reopened.close().await;
    assert_eq!(user_version(&db).await, 5);
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

/// True if `contract_claims` carries the `fingerprint_version` column Step 4
/// adds. Asserted on upgrade so a missing ALTER is caught here, not silently
/// when a claim decodes under the wrong version later.
async fn claim_has_fingerprint_version_column(db: &Path) -> bool {
    let pool = read_pool(db).await;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info('contract_claims') WHERE name='fingerprint_version'")
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
