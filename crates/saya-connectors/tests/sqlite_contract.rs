use saya_config::MapSecretResolver;
use saya_connectors::{ConnectorOptions, DatabaseConnector, SqliteConnector, build_connector};
use saya_types::{
    BoundParam, ConnectionError, DatabaseProfile, ForeignKey, ParamType, ParamValue, QueryRequest,
    QueryResult,
};
use serde_json::Value;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::path::Path;

async fn create_test_fixture(path: &Path) {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await.unwrap();

    sqlx::query(
        "CREATE TABLE t (
            id INTEGER,
            amount REAL,
            label TEXT,
            blob_col BLOB,
            maybe TEXT,
            gen INTEGER GENERATED ALWAYS AS (id * 2) VIRTUAL,
            no_type
        );",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("CREATE VIEW v AS SELECT id FROM t;")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(r#"CREATE TABLE "weird ""name""" (x INTEGER);"#)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO t (id, amount, label, blob_col, maybe, no_type)
         VALUES (1, 3.5, 'hello', x'00ff10', NULL, 'untyped_val');",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO t (id, amount, label, blob_col, maybe, no_type)
         VALUES (2, 2.718, 'world', x'1234', 'populated', 42);",
    )
    .execute(&pool)
    .await
    .unwrap();

    pool.close().await;
}

async fn create_param_fixture(path: &Path) {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await.unwrap();

    sqlx::query(
        "CREATE TABLE params_t (
            id INTEGER PRIMARY KEY,
            amount REAL,
            label TEXT,
            day TEXT,
            seen_at TEXT
        );",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO params_t (id, amount, label, day, seen_at)
         VALUES (1, 12.5, 'alpha', '2024-02-29', '2024-02-29T03:04:05Z'),
                (2, 3.25, 'beta', '2024-01-01', '2024-01-01T00:00:00Z');",
    )
    .execute(&pool)
    .await
    .unwrap();

    pool.close().await;
}

async fn open_param_connector(path: &Path) -> SqliteConnector {
    SqliteConnector::open(path, true, ConnectorOptions::default())
        .await
        .expect("fixture opens")
}

fn bound(name: &str, value: ParamValue) -> BoundParam {
    BoundParam {
        name: name.to_owned(),
        value,
    }
}

fn only_row(result: &QueryResult) -> &Vec<Value> {
    match result.rows.first() {
        Some(Value::Array(cells)) => cells,
        other => panic!("expected one array row, got {other:?}"),
    }
}

#[path = "sqlite_contract/connection.rs"]
mod connection;
#[path = "sqlite_contract/connection_timeout.rs"]
mod connection_timeout;
#[path = "sqlite_contract/diagnostics.rs"]
mod diagnostics;
#[path = "sqlite_contract/foreign_keys.rs"]
mod foreign_keys;
#[path = "sqlite_contract/parameter_order.rs"]
mod parameter_order;
#[path = "sqlite_contract/parameter_values.rs"]
mod parameter_values;
#[path = "sqlite_contract/result_budget.rs"]
mod result_budget;
#[path = "sqlite_contract/schema_nullability.rs"]
mod schema_nullability;
