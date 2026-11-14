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

include!("demo_tests/calendar.rs");

include!("demo_tests/fixture_data.rs");

include!("demo_tests/connections.rs");

include!("demo_tests/lifecycle.rs");
