use std::{
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use saya_connectors::{ConnectorOptions, DatabaseConnector, MySqlConnector};
use saya_types::QueryRequest;
use sqlx::{
    MySqlPool,
    mysql::{MySqlConnectOptions, MySqlPoolOptions},
};

#[tokio::test]
async fn localhost_mysql_fixture_round_trips_connector_contract() {
    let Ok(url) = std::env::var("SAYA_TEST_MYSQL_URL") else {
        return;
    };
    let options = MySqlConnectOptions::from_str(&url).expect("SAYA_TEST_MYSQL_URL must be valid");
    let database = options
        .get_database()
        .expect("SAYA_TEST_MYSQL_URL must select a database")
        .to_owned();
    let table = format!("saya_fixture_{}_{}", std::process::id(), nonce());
    let pool = MySqlPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    setup(&pool, &table).await;
    let connector = MySqlConnector::from_options(options, &database, ConnectorOptions::default());
    connector.connect().await.unwrap();
    let schema = connector.schema().await.unwrap();
    assert_eq!(schema.databases[0].name, "MySQL");
    assert_eq!(schema.databases[0].schemas[0].name, database);
    let metadata = schema.databases[0].schemas[0]
        .tables
        .iter()
        .find(|item| item.name == table)
        .unwrap();
    assert!(
        metadata
            .columns
            .iter()
            .any(|item| item.name == "event_year")
    );
    assert!(metadata.columns.iter().any(|item| item.name == "flags"));
    let result = connector.execute(QueryRequest::new(format!("SELECT id, amount, payload, event_date, event_time, event_at, captured_at, event_year, unsigned_value, flags, bytes, note FROM `{table}` ORDER BY id"), 1)).await.unwrap();
    assert!(result.truncated);
    assert_eq!(
        result.rows,
        vec![
            serde_json::json!([1, "12.34", {"x": 1}, "2024-01-02", "03:04:05", "2024-01-02 03:04:05", "2024-01-02 03:04:05", 2024, 42, 5, "00ff", null])
        ]
    );
    sqlx::query(&format!("DROP TABLE `{table}`"))
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

async fn setup(pool: &MySqlPool, table: &str) {
    let sql = format!(
        "CREATE TABLE `{table}` (id INT, amount DECIMAL(4,2), payload JSON, event_date DATE, event_time TIME, event_at DATETIME, captured_at TIMESTAMP NULL, event_year YEAR, unsigned_value BIGINT UNSIGNED, flags BIT(8), bytes BLOB, note VARCHAR(16) NULL)"
    );
    sqlx::query(&sql).execute(pool).await.unwrap();
    sqlx::query(&format!("INSERT INTO `{table}` VALUES (1, 12.34, JSON_OBJECT('x', 1), '2024-01-02', '03:04:05', '2024-01-02 03:04:05', '2024-01-02 03:04:05', 2024, 42, b'00000101', X'00FF', NULL), (2, 99.99, JSON_OBJECT('x', 2), '2024-01-03', '03:04:06', '2024-01-03 03:04:06', '2024-01-03 03:04:06', 2025, 43, b'00000001', X'01', 'extra')")).execute(pool).await.unwrap();
}

fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}
