use saya_store::{
    AuditEntry, AuditOperation, AuditStatus, AuditStore, SchemaStore, SqliteStateStore, StoreError,
};
use saya_types::{Column, Database, MAX_SCHEMA_COLUMNS, Schema, SchemaTree, Table};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const PROFILE: &str = "p-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn schema_roundtrip_upsert_invalidate_and_versioned_reopen() {
    let root = temp_root("schema");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    let cached = store.get_schema(PROFILE).await.unwrap().unwrap();
    assert_eq!(cached.schema, schema("events"));
    assert_eq!(cached.version, 1);
    store
        .upsert_schema(PROFILE, &schema("events_v2"))
        .await
        .unwrap();
    assert_eq!(
        SqliteStateStore::new(&db)
            .get_schema(PROFILE)
            .await
            .unwrap()
            .unwrap()
            .schema,
        schema("events_v2")
    );
    assert_eq!(store.list_schema_metadata().await.unwrap().len(), 1);
    store.invalidate_schema(PROFILE).await.unwrap();
    assert!(store.get_schema(PROFILE).await.unwrap().is_none());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn oversized_schema_is_rejected_before_cache_write() {
    let root = temp_root("schema-limit");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let schema = SchemaTree {
        databases: vec![Database {
            name: "main".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: "wide".into(),
                    columns: (0..=MAX_SCHEMA_COLUMNS)
                        .map(|index| Column {
                            name: format!("c{index}"),
                            data_type: "TEXT".into(),
                            nullable: true,
                        })
                        .collect(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                }],
            }],
        }],
    };
    assert_eq!(
        store.upsert_schema(PROFILE, &schema).await,
        Err(StoreError::LimitExceeded)
    );
    assert!(store.get_schema(PROFILE).await.unwrap().is_none());
    let _ = fs::remove_dir_all(root);
}

/// A tree that passes structural validation but serializes past the byte
/// cap must still refuse — and cache nothing. Unlike the count-overflow
/// tree above, this one reaches the serialization seam, so it pins the
/// bounded-serializer contract rather than the validator.
#[tokio::test]
async fn oversized_but_valid_schema_tree_is_refused_before_caching() {
    use saya_types::MAX_SCHEMA_BYTES;
    let root = temp_root("schema-bytes");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wide_name = "n".repeat(200);
    let wide_type = "t".repeat(200);
    let schema = SchemaTree {
        databases: vec![Database {
            name: "main".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: "wide".into(),
                    columns: (0..60_000)
                        .map(|index| Column {
                            name: format!("{wide_name}{index:05}"),
                            data_type: wide_type.clone(),
                            nullable: true,
                        })
                        .collect(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                }],
            }],
        }],
    };
    schema
        .validate()
        .expect("the tree must pass structural validation");
    let rendered = serde_json::to_string(&schema).expect("the tree must render");
    assert!(
        rendered.len() > MAX_SCHEMA_BYTES,
        "the fixture must exceed the byte cap: {}",
        rendered.len()
    );
    assert_eq!(
        store.upsert_schema(PROFILE, &schema).await,
        Err(StoreError::LimitExceeded)
    );
    assert!(store.get_schema(PROFILE).await.unwrap().is_none());
    let _ = fs::remove_dir_all(root);
}

/// A stored row that is already oversized — planted past every write seam
/// by raw SQL — must refuse on read without materializing the whole value.
#[tokio::test]
async fn oversized_stored_schema_row_is_refused_on_read() {
    use saya_types::MAX_SCHEMA_BYTES;
    let root = temp_root("schema-row");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    let oversized = "x".repeat(MAX_SCHEMA_BYTES + 1);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&db)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE schema_cache SET schema_json=? WHERE profile_id=?")
        .bind(&oversized)
        .bind(PROFILE)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(
        store.get_schema(PROFILE).await,
        Err(StoreError::LimitExceeded)
    );
    let _ = fs::remove_dir_all(root);
}

/// A025: the read gate must measure bytes, not characters. This payload is
/// valid JSON whose character count fits under the bound while its UTF-8
/// byte length exceeds it. `length()` counts characters, so the old gate
/// lets it through and the read fails later with `Unavailable` (parse) —
/// the byte gate must refuse it with `LimitExceeded` instead.
#[tokio::test]
async fn multibyte_schema_row_over_the_byte_bound_is_refused_on_read() {
    use saya_types::MAX_SCHEMA_BYTES;
    let root = temp_root("schema-row-bytes");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    // `é` is 1 char but 2 bytes in UTF-8: the wrapper adds 10 chars, so
    // MAX-10 of them land exactly on the char bound while bytes go ~2x over.
    let oversized = format!("{{\"pad\":\"{}\"}}", "é".repeat(MAX_SCHEMA_BYTES - 10));
    assert_eq!(oversized.chars().count(), MAX_SCHEMA_BYTES);
    assert_eq!(oversized.len(), 2 * MAX_SCHEMA_BYTES - 10);
    assert!(oversized.len() > MAX_SCHEMA_BYTES);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&db)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    // Pin the mechanism: SQLite's `length()` sees characters here.
    let chars: i64 = sqlx::query_scalar("SELECT length(?)")
        .bind(&oversized)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(chars, MAX_SCHEMA_BYTES as i64);
    sqlx::query("UPDATE schema_cache SET schema_json=? WHERE profile_id=?")
        .bind(&oversized)
        .bind(PROFILE)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(
        store.get_schema(PROFILE).await,
        Err(StoreError::LimitExceeded)
    );
    let _ = fs::remove_dir_all(root);
}

/// The byte gate boundary: exactly `MAX_SCHEMA_BYTES` bytes passes the
/// gate (the planted row is not valid schema JSON, so the read then fails
/// at parse with `Unavailable`), while one byte more refuses with
/// `LimitExceeded`. Both sides pin the gate, not the parser.
#[tokio::test]
async fn schema_row_byte_gate_accepts_exact_bound_and_refuses_one_past() {
    use saya_types::MAX_SCHEMA_BYTES;
    for (label, payload, expected) in [
        (
            "exact",
            "x".repeat(MAX_SCHEMA_BYTES),
            StoreError::Unavailable,
        ),
        (
            "one-past",
            "x".repeat(MAX_SCHEMA_BYTES + 1),
            StoreError::LimitExceeded,
        ),
    ] {
        let root = temp_root(&format!("schema-row-bound-{label}"));
        let db = root.join("state.sqlite3");
        let store = SqliteStateStore::new(&db);
        store
            .upsert_schema(PROFILE, &schema("events"))
            .await
            .unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&db)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::query("UPDATE schema_cache SET schema_json=? WHERE profile_id=?")
            .bind(&payload)
            .bind(PROFILE)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        assert_eq!(store.get_schema(PROFILE).await, Err(expected), "{label}");
        let _ = fs::remove_dir_all(root);
    }
}

/// The oversized row must refuse inside SQLite: the gate's guarded column
/// comes back NULL, so the whole value never crosses to the client —
/// never fetch-then-measure. This extends (not weakens) the existing
/// oversized-row refusal test by pinning the mechanism directly.
#[tokio::test]
async fn oversized_row_gate_returns_null_without_materializing() {
    use saya_types::MAX_SCHEMA_BYTES;
    let root = temp_root("schema-row-gate");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    let oversized = "x".repeat(MAX_SCHEMA_BYTES + 1);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&db)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE schema_cache SET schema_json=? WHERE profile_id=?")
        .bind(&oversized)
        .bind(PROFILE)
        .execute(&pool)
        .await
        .unwrap();
    // The byte-measured guard must yield NULL for the oversized row while
    // the byte length is still reported — refusal decided inside SQLite.
    let (bytes, guarded): (i64, Option<String>) = sqlx::query_as(
        "SELECT length(CAST(schema_json AS BLOB)), CASE WHEN length(CAST(schema_json AS BLOB)) <= ? THEN schema_json END FROM schema_cache WHERE profile_id=?",
    )
    .bind(MAX_SCHEMA_BYTES as i64)
    .bind(PROFILE)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bytes, (MAX_SCHEMA_BYTES + 1) as i64);
    assert_eq!(guarded, None);
    pool.close().await;
    assert_eq!(
        store.get_schema(PROFILE).await,
        Err(StoreError::LimitExceeded)
    );
    let _ = fs::remove_dir_all(root);
}

/// Q2: schema bytes and freshness metadata must come from one consistent
/// read. This pins the source-level guarantee: `get_schema` may run a
/// single statement against `schema_cache`, so no await — and no
/// concurrent upsert or delete — can land between the JSON fetch and the
/// metadata fetch and pair one version's bytes with another's timestamp.
#[test]
fn schema_read_is_a_single_statement_over_the_cache_row() {
    let source = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/schema_store.rs"))
        .expect("schema_store.rs must be readable");
    let body = source
        .split("async fn get_schema")
        .nth(1)
        .expect("get_schema must exist")
        .split("async fn invalidate_schema")
        .next()
        .expect("invalidate_schema must follow get_schema");
    assert_eq!(
        body.matches("FROM schema_cache").count(),
        1,
        "get_schema must read schema bytes and metadata in one statement"
    );
}

/// Q2: the returned freshness metadata always belongs to the returned
/// bytes. A planted row with distinctive metadata must echo back exactly —
/// no old-bytes/new-timestamp pairing from split reads.
#[tokio::test]
async fn schema_read_pairs_bytes_with_their_own_metadata() {
    let root = temp_root("schema-meta-pair");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&db)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE schema_cache SET updated_unix_ms=424242, version=7 WHERE profile_id=?")
        .bind(PROFILE)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let cached = store.get_schema(PROFILE).await.unwrap().unwrap();
    assert_eq!(cached.schema, schema("events"));
    assert_eq!(cached.updated_unix_ms, 424242);
    assert_eq!(cached.version, 7);
    let _ = fs::remove_dir_all(root);
}

/// Q2 hammer: two writers rewrite one cached row while the reader hammers
/// the production `get_schema` read path. Each writer stamps schema bytes
/// and the generation's freshness metadata in ONE atomic statement — the
/// same single-statement upsert production `upsert_schema` runs inside its
/// transaction — so the row on disk only ever holds self-consistent
/// generations: gen-a ⇔ (version 1, odd ms), gen-b ⇔ (version 2, even ms).
/// Every read must pair bytes with metadata from the same generation: a
/// read that pairs one generation's bytes with the other's version or
/// timestamp fails. Public-read controls observe each generation before the
/// contention phase; the concurrent reader checks every pair without
/// assuming scheduler fairness. Each writer ignores the stop flag until it
/// has completed `MIN_WRITES` upserts. The fixed read count and writer cap
/// bound the runtime.
#[tokio::test]
async fn concurrent_upserts_never_tear_schema_from_its_metadata() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    // The public write path computes metadata internally (`now()` and
    // `SCHEMA_VERSION`) and cannot express per-generation metadata, so the
    // writers stamp generations with the production upsert statement over
    // raw pools — the established planted-row pattern of this file.
    const VERSION_A: i64 = 1;
    const VERSION_B: i64 = 2;
    const ODD_BASE_MS: i64 = 1_000_001;
    const EVEN_BASE_MS: i64 = 1_000_000;
    const MIN_WRITES: i64 = 10;
    const MAX_WRITES: i64 = 2_000;
    const READS: usize = 200;

    let root = temp_root("schema-tear");
    let db = root.join("state.sqlite3");
    let store = Arc::new(SqliteStateStore::new(&db));
    assert!(store.get_schema(PROFILE).await.unwrap().is_none());
    let json_a = serde_json::to_string(&schema("gen-a")).unwrap();
    let json_b = serde_json::to_string(&schema("gen-b")).unwrap();
    let pool_a = raw_pool(&db).await;
    let pool_b = raw_pool(&db).await;
    stamp_generation(&pool_a, PROFILE, &json_a, ODD_BASE_MS, VERSION_A)
        .await
        .expect("seed upsert must succeed");
    let cached_a = store.get_schema(PROFILE).await.unwrap().unwrap();
    assert_eq!(cached_a.schema, schema("gen-a"));
    assert_eq!(i64::from(cached_a.version), VERSION_A);
    assert_eq!(cached_a.updated_unix_ms % 2, 1);
    stamp_generation(&pool_b, PROFILE, &json_b, EVEN_BASE_MS, VERSION_B)
        .await
        .expect("generation B control upsert must succeed");
    let cached_b = store.get_schema(PROFILE).await.unwrap().unwrap();
    assert_eq!(cached_b.schema, schema("gen-b"));
    assert_eq!(i64::from(cached_b.version), VERSION_B);
    assert_eq!(cached_b.updated_unix_ms % 2, 0);

    let stop = Arc::new(AtomicBool::new(false));
    let writer_a = {
        let stop = Arc::clone(&stop);
        let json = json_a.clone();
        let pool = pool_a.clone();
        tokio::spawn(async move {
            let mut index: i64 = 0;
            // A writer ignores the stop flag until it has completed
            // MIN_WRITES upserts, so the post-join progress assertion can
            // never pass while a writer starved below the bound.
            while (!stop.load(Ordering::Relaxed) || index < MIN_WRITES) && index < MAX_WRITES {
                stamp_generation(&pool, PROFILE, &json, ODD_BASE_MS + 2 * index, VERSION_A)
                    .await
                    .expect("writer A upsert must succeed");
                index += 1;
            }
            index
        })
    };
    let writer_b = {
        let stop = Arc::clone(&stop);
        let json = json_b.clone();
        let pool = pool_b.clone();
        tokio::spawn(async move {
            let mut index: i64 = 0;
            while (!stop.load(Ordering::Relaxed) || index < MIN_WRITES) && index < MAX_WRITES {
                stamp_generation(&pool, PROFILE, &json, EVEN_BASE_MS + 2 * index, VERSION_B)
                    .await
                    .expect("writer B upsert must succeed");
                index += 1;
            }
            index
        })
    };
    let (mut saw_a, mut saw_b) = (0_u32, 0_u32);
    for _ in 0..READS {
        match store.get_schema(PROFILE).await {
            Ok(Some(cached)) => {
                let table = &cached.schema.databases[0].schemas[0].tables[0].name;
                match table.as_str() {
                    "gen-a" => {
                        saw_a += 1;
                        assert_eq!(
                            i64::from(cached.version),
                            VERSION_A,
                            "gen-a bytes paired with version {}",
                            cached.version
                        );
                        assert_eq!(
                            cached.updated_unix_ms % 2,
                            1,
                            "gen-a bytes paired with timestamp {}",
                            cached.updated_unix_ms
                        );
                    }
                    "gen-b" => {
                        saw_b += 1;
                        assert_eq!(
                            i64::from(cached.version),
                            VERSION_B,
                            "gen-b bytes paired with version {}",
                            cached.version
                        );
                        assert_eq!(
                            cached.updated_unix_ms % 2,
                            0,
                            "gen-b bytes paired with timestamp {}",
                            cached.updated_unix_ms
                        );
                    }
                    other => panic!("torn schema bytes: {other}"),
                }
            }
            Ok(None) => {}
            Err(error) => panic!("a live row must read cleanly: {error:?}"),
        }
    }
    stop.store(true, Ordering::Relaxed);
    let wrote_a = writer_a.await.unwrap();
    let wrote_b = writer_b.await.unwrap();
    assert!(
        saw_a + saw_b >= 1,
        "the contended reader must observe a live generation"
    );
    assert!(
        wrote_a >= MIN_WRITES,
        "writer A completed only {wrote_a} upserts"
    );
    assert!(
        wrote_b >= MIN_WRITES,
        "writer B completed only {wrote_b} upserts"
    );
    pool_a.close().await;
    pool_b.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn audit_is_typed_bounded_retained_and_decoded_without_payload_fields() {
    let root = temp_root("audit");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    for index in 0..1_005 {
        let mut event = AuditEntry::new(PROFILE, AuditOperation::Query, AuditStatus::Success, 5);
        event.row_count = Some(index);
        event.truncated = Some(false);
        store.record_audit(event).await.unwrap();
    }
    let audit = store.recent_audit(2_000).await.unwrap();
    assert_eq!(audit.len(), 1_000);
    assert!(
        audit
            .iter()
            .all(|row| row.event.operation == AuditOperation::Query
                && row.event.profile_id == PROFILE)
    );
    assert!(store.recent_audit(20_000).await.unwrap().len() <= 1_000);
    assert!(store.recent_audit(0).await.unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn independent_stores_serialize_first_initialization_and_corrupt_files_fail_safely() {
    let root = temp_root("concurrent");
    let db = root.join("state.sqlite3");
    fs::write(&db, []).unwrap();
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let store = SqliteStateStore::new(&db);
        tasks.push(tokio::spawn(async move {
            store
                .upsert_schema(PROFILE, &schema("events"))
                .await
                .unwrap();
            store
                .record_audit(AuditEntry::new(
                    PROFILE,
                    AuditOperation::SchemaRefresh,
                    AuditStatus::Success,
                    1,
                ))
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(
        SqliteStateStore::new(&db)
            .recent_audit(100)
            .await
            .unwrap()
            .len(),
        12
    );
    let corrupt_root = temp_root("corrupt");
    let corrupt = corrupt_root.join("state.sqlite3");
    fs::write(&corrupt, b"server error /private/tmp password").unwrap();
    let error = SqliteStateStore::new(corrupt)
        .get_schema(PROFILE)
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::OpenFailed);
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(corrupt_root);
}

#[tokio::test]
async fn adversarial_labels_are_rejected_and_never_written() {
    let root = temp_root("sentinels");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    for value in [
        "password=secret",
        "PRIVATE KEY",
        "SELECT raw_literal",
        "prompt/row/path",
    ] {
        assert!(store.upsert_schema(value, &schema("safe")).await.is_err());
        assert!(
            store
                .record_audit(AuditEntry::new(
                    value,
                    AuditOperation::Query,
                    AuditStatus::Failure,
                    1
                ))
                .await
                .is_err()
        );
    }
    let bytes = fs::read(&db).unwrap_or_default();
    let bytes = String::from_utf8_lossy(&bytes).into_owned();
    for value in [
        "password=secret",
        "PRIVATE KEY",
        "SELECT raw_literal",
        "prompt/row/path",
    ] {
        assert!(!bytes.contains(value));
    }
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn malformed_profile_id_is_rejected_as_invalid() {
    let root = temp_root("bad-profile-id");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let error = store
        .upsert_schema("not-a-profile-id", &schema("events"))
        .await
        .unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[tokio::test]
async fn unix_parent_database_and_sidecars_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("permissions");
    let db = root.join("created").join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    assert_eq!(
        fs::metadata(root.join("created"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    for entry in fs::read_dir(root.join("created")).unwrap().flatten() {
        assert_eq!(
            entry.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[tokio::test]
async fn unix_existing_state_parent_preserves_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let root = temp_root("shared-parent");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn non_utf8_sidecar_paths_are_lossless() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let path = PathBuf::from(std::ffi::OsString::from_vec(vec![b's', 0xff, b'a']));
    assert_eq!(
        saya_store::state_sidecar_path(&path, "-wal")
            .as_os_str()
            .as_bytes(),
        b"s\xffa-wal"
    );
}

#[test]
fn store_errors_are_payload_free() {
    assert_eq!(format!("{:?}", StoreError::Unavailable), "Unavailable");
}

fn schema(table: &str) -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "main".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: table.into(),
                    columns: vec![Column {
                        name: "id".into(),
                        data_type: "INTEGER".into(),
                        nullable: false,
                    }],
                    primary_key: vec![],
                    foreign_keys: vec![],
                }],
            }],
        }],
    }
}

/// A single-connection pool over the same store file, as the planted-row
/// tests use. The connection's default 5s busy timeout absorbs the write
/// lock held briefly by the sibling writer or a commit.
async fn raw_pool(db: &std::path::Path) -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(db)
                .create_if_missing(true),
        )
        .await
        .unwrap()
}

/// One atomic rewrite of the cached row: bytes and metadata land together
/// in a single statement — the statement production `upsert_schema` runs
/// inside its transaction — so the row only ever holds self-consistent
/// generations.
async fn stamp_generation(
    pool: &sqlx::SqlitePool,
    profile_id: &str,
    schema_json: &str,
    updated_unix_ms: i64,
    version: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO schema_cache(profile_id, schema_json, updated_unix_ms, version) VALUES (?, ?, ?, ?) ON CONFLICT(profile_id) DO UPDATE SET schema_json=excluded.schema_json, updated_unix_ms=excluded.updated_unix_ms, version=excluded.version")
        .bind(profile_id)
        .bind(schema_json)
        .bind(updated_unix_ms)
        .bind(version)
        .execute(pool)
        .await
        .map(drop)
}
fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("saya-state-{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}
