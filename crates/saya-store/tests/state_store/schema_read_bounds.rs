use super::*;

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
