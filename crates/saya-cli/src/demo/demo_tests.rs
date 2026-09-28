#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{fs, path::Path, path::PathBuf};

use saya_config::ConnectionsFile;
use saya_types::DatabaseProfile;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

use super::fixture::{self, FixtureOutcome};

fn temp_demo_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("saya-demo-unit-{}-{label}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

async fn connect_read_only(db: &Path) -> SqlitePool {
    let options = SqliteConnectOptions::new().filename(db).read_only(true);
    SqlitePool::connect_with(options).await.unwrap()
}

async fn scalar(db: &Path, sql: &str) -> i64 {
    let pool = connect_read_only(db).await;
    let (value,): (i64,) = sqlx::query_as(sql).fetch_one(&pool).await.unwrap();
    pool.close().await;
    value
}

async fn dump(db: &Path) -> Vec<String> {
    let pool = connect_read_only(db).await;
    let mut dump = Vec::new();
    for (table, sql) in [
        (
            "customers",
            "SELECT id || '|' || name || '|' || COALESCE(email, '<null>') || '|' || region \
             || '|' || signup_date || '|' || status FROM customers ORDER BY id",
        ),
        (
            "orders",
            "SELECT id || '|' || customer_id || '|' || order_date \
             || '|' || COALESCE(CAST(amount_cents AS TEXT), '<null>') || '|' || status \
             FROM orders ORDER BY id",
        ),
        (
            "customer_contacts",
            "SELECT customer_id || '|' || channel || '|' || value FROM customer_contacts \
             ORDER BY customer_id, channel, value",
        ),
        (
            "saya_demo_meta",
            "SELECT key || '|' || value FROM saya_demo_meta ORDER BY key",
        ),
    ] {
        let rows: Vec<(String,)> = sqlx::query_as(sql).fetch_all(&pool).await.unwrap();
        for (line,) in rows {
            dump.push(format!("{table}|{line}"));
        }
    }
    pool.close().await;
    dump
}

#[tokio::test]
async fn iso_date_anchors_at_2026_01_01() {
    use super::calendar::iso_date;
    assert_eq!(iso_date(0), "2026-01-01");
    assert_eq!(
        iso_date(90),
        "2025-10-03",
        "the last 90 days start past 2025-10-03"
    );
    assert_eq!(iso_date(366), "2024-12-31");
}

#[tokio::test]
async fn fixture_is_deterministic() {
    let dir_a = temp_demo_dir("det-a");
    let dir_b = temp_demo_dir("det-b");
    fixture::ensure(&dir_a, false).await.unwrap();
    fixture::ensure(&dir_b, false).await.unwrap();
    let dump_a = dump(&fixture::db_path(&dir_a)).await;
    let dump_b = dump(&fixture::db_path(&dir_b)).await;
    assert!(!dump_a.is_empty(), "the dump covers every table");
    assert_eq!(
        dump_a, dump_b,
        "two builds must produce identical table contents"
    );
    let _ = fs::remove_dir_all(&dir_a);
    let _ = fs::remove_dir_all(&dir_b);
}

#[tokio::test]
async fn fixture_has_the_traps() {
    let dir = temp_demo_dir("traps");
    fixture::ensure(&dir, false).await.unwrap();
    let db = fixture::db_path(&dir);

    let multi_contact = scalar(
        &db,
        "SELECT count(*) FROM (SELECT customer_id FROM customer_contacts \
         GROUP BY customer_id HAVING count(*) >= 2)",
    )
    .await;
    assert!(
        multi_contact >= 1,
        "some customer must have 2+ contact rows (duplicate-join trap)"
    );
    assert!(
        scalar(&db, "SELECT count(*) FROM customers WHERE email IS NULL").await >= 1,
        "some customer email must be NULL"
    );
    assert!(
        scalar(
            &db,
            "SELECT count(*) FROM orders WHERE amount_cents IS NULL"
        )
        .await
            >= 1,
        "some order amount must be NULL"
    );
    assert!(
        scalar(
            &db,
            "SELECT count(*) FROM orders WHERE order_date = '2025-12-31'"
        )
        .await
            >= 1,
        "orders must hit the 2025-12-31 boundary"
    );
    assert!(
        scalar(
            &db,
            "SELECT count(*) FROM orders WHERE order_date = '2026-01-01'"
        )
        .await
            >= 1,
        "orders must hit the 2026-01-01 boundary"
    );
    let stale_active = scalar(
        &db,
        "SELECT count(*) FROM customers c WHERE c.status = 'active' AND NOT EXISTS \
         (SELECT 1 FROM orders o WHERE o.customer_id = c.id AND o.order_date > '2025-10-03')",
    )
    .await;
    assert!(
        stale_active >= 1,
        "an active customer must not have ordered in the last 90 days of the data"
    );
    let recent_inactive = scalar(
        &db,
        "SELECT count(*) FROM customers c WHERE c.status <> 'active' AND EXISTS \
         (SELECT 1 FROM orders o WHERE o.customer_id = c.id AND o.order_date > '2025-10-03')",
    )
    .await;
    assert!(
        recent_inactive >= 1,
        "recency and status must disagree in the other direction too"
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM customers WHERE email IS NOT NULL \
             AND email NOT LIKE '%@example.invalid'"
        )
        .await,
        0,
        "emails must be obviously fake"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn fixture_row_and_size_bounds() {
    let dir = temp_demo_dir("bounds");
    fixture::ensure(&dir, false).await.unwrap();
    let db = fixture::db_path(&dir);
    let counts = [
        (
            "customers",
            scalar(&db, "SELECT count(*) FROM customers").await,
        ),
        ("orders", scalar(&db, "SELECT count(*) FROM orders").await),
        (
            "customer_contacts",
            scalar(&db, "SELECT count(*) FROM customer_contacts").await,
        ),
    ];
    for (table, count) in counts {
        assert!(count > 0, "{table} must be populated");
        assert!(count <= 1000, "{table} must stay within 1000 rows: {count}");
    }
    let size = fs::metadata(&db).unwrap().len();
    assert!(
        size <= 5 * 1024 * 1024,
        "the demo file must stay within 5 MiB: {size}"
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM saya_demo_meta \
             WHERE key = 'fixture_version' AND value = '1'"
        )
        .await,
        1,
        "the meta table records fixture_version = 1"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn connections_toml_is_a_single_private_demo_profile() {
    let dir = temp_demo_dir("connections");
    fixture::ensure(&dir, false).await.unwrap();
    let connections = fs::read_to_string(fixture::connections_path(&dir)).unwrap();
    let parsed = ConnectionsFile::from_toml(&connections)
        .unwrap_or_else(|error| panic!("saya must parse the written connections file: {error}"));
    let expected = DatabaseProfile::Sqlite {
        path: fixture::db_path(&dir).display().to_string(),
        read_only: true,
    };
    let profiles: Vec<(String, DatabaseProfile)> = parsed.profiles.into_iter().collect();
    assert_eq!(
        profiles,
        vec![("demo".to_owned(), expected)],
        "exactly one demo profile pointing at the demo database"
    );
    #[cfg(unix)]
    {
        let conn_mode = fs::metadata(fixture::connections_path(&dir))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(conn_mode, 0o600, "the connections file is 0600");
        let db_mode = fs::metadata(fixture::db_path(&dir))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(db_mode, 0o600, "the demo database is 0600");
        let dir_mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "the demo directory is 0700");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn reuse_keeps_the_database_file() {
    let dir = temp_demo_dir("reuse");
    assert!(matches!(
        fixture::ensure(&dir, false).await.unwrap(),
        FixtureOutcome::Built
    ));
    let before = fs::metadata(fixture::db_path(&dir))
        .unwrap()
        .modified()
        .unwrap();
    assert!(matches!(
        fixture::ensure(&dir, false).await.unwrap(),
        FixtureOutcome::Reused
    ));
    let after = fs::metadata(fixture::db_path(&dir))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(before, after, "reuse must not rewrite the database file");
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn reset_rebuilds_the_fixture() {
    let dir = temp_demo_dir("reset");
    fixture::ensure(&dir, false).await.unwrap();
    assert!(matches!(
        fixture::ensure(&dir, true).await.unwrap(),
        FixtureOutcome::Built
    ));
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn missing_or_corrupt_database_is_rebuilt() {
    let dir = temp_demo_dir("corrupt");
    fs::create_dir_all(&dir).unwrap();
    fs::write(fixture::db_path(&dir), b"not a sqlite database").unwrap();
    assert!(
        matches!(
            fixture::ensure(&dir, false).await.unwrap(),
            FixtureOutcome::Built
        ),
        "a corrupt or wrong-version file is rebuilt, not reused"
    );
    assert!(matches!(
        fixture::ensure(&dir, false).await.unwrap(),
        FixtureOutcome::Reused
    ));
    let _ = fs::remove_dir_all(&dir);
}
