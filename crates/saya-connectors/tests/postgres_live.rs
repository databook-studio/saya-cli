use bigdecimal::BigDecimal;
use chrono::NaiveDateTime;
use saya_connectors::{ConnectorOptions, DatabaseConnector, PostgresConnector};
use saya_types::{ForeignKey, QueryRequest};
use sqlx::{PgPool, types::Json};
use std::str::FromStr;

#[tokio::test]
async fn live_postgres_executes_and_marks_truncation() {
    let Ok(url) = std::env::var("SAYA_TEST_POSTGRES_URL") else {
        return;
    };
    let setup = PgPool::connect(&url).await.unwrap();
    sqlx::query("DROP TABLE IF EXISTS saya_cli_live_fixture")
        .execute(&setup)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE saya_cli_live_fixture (id INTEGER PRIMARY KEY, amount NUMERIC, optional TEXT, payload JSONB, occurred_at TIMESTAMP)")
        .execute(&setup)
        .await
        .unwrap();
    let timestamp = NaiveDateTime::parse_from_str("2024-01-02 03:04:05", "%F %T").unwrap();
    sqlx::query("INSERT INTO saya_cli_live_fixture (id, amount, optional, payload, occurred_at) VALUES ($1, $2, $3, $4, $5), ($6, $7, $8, $9, $10)")
        .bind(1_i32)
        .bind(BigDecimal::from_str("123.4500").unwrap())
        .bind(Option::<String>::None)
        .bind(Json(serde_json::json!({"source": "ci"})))
        .bind(timestamp)
        .bind(2_i32)
        .bind(BigDecimal::from_str("999.0001").unwrap())
        .bind("present")
        .bind(Json(serde_json::json!({"source": "ci"})))
        .bind(timestamp)
        .execute(&setup)
        .await
        .unwrap();
    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap();
    let connector = PostgresConnector::from_options(options, ConnectorOptions::default());
    connector.connect().await.unwrap();
    let result = connector
        .execute(QueryRequest::new(
            "SELECT amount, optional, payload, occurred_at, 42::oid AS object_id FROM saya_cli_live_fixture ORDER BY id",
            1,
        ))
        .await
        .unwrap();
    assert_eq!(
        result.columns,
        vec!["amount", "optional", "payload", "occurred_at", "object_id"]
    );
    assert_eq!(result.row_count, 1);
    assert!(result.truncated);
    assert_eq!(result.rows[0][0], "123.4500");
    assert!(result.rows[0][1].is_null());
    assert_eq!(result.rows[0][2], serde_json::json!({"source": "ci"}));
    assert_eq!(result.rows[0][3], "2024-01-02 03:04:05");
    assert_eq!(result.rows[0][4], 42);
    let schema = connector.schema().await.unwrap();
    let columns = schema.databases[0]
        .schemas
        .iter()
        .flat_map(|schema| schema.tables.iter())
        .find(|table| table.name == "saya_cli_live_fixture")
        .unwrap()
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        vec!["id", "amount", "optional", "payload", "occurred_at"]
    );
    sqlx::query("DROP TABLE saya_cli_live_fixture")
        .execute(&setup)
        .await
        .unwrap();
}

/// Exercises the `pg_constraint` foreign-key path against a live server: a
/// single-column reference, a composite reference, and a self-reference must
/// all come back with their columns in declaration order. Gated behind a URL
/// so it runs only where a PostgreSQL instance is reachable.
#[tokio::test]
async fn live_postgres_discovers_foreign_keys() {
    let Ok(url) = std::env::var("SAYA_TEST_POSTGRES_URL") else {
        return;
    };
    let setup = PgPool::connect(&url).await.unwrap();
    for stmt in [
        "DROP TABLE IF EXISTS saya_fk_child",
        "DROP TABLE IF EXISTS saya_fk_parent",
        "DROP TABLE IF EXISTS saya_fk_self",
        "CREATE TABLE saya_fk_parent (a INTEGER, b INTEGER, PRIMARY KEY (a, b))",
        "CREATE TABLE saya_fk_child (id INTEGER PRIMARY KEY, a INTEGER, b INTEGER, FOREIGN KEY (a, b) REFERENCES saya_fk_parent(a, b))",
        "CREATE TABLE saya_fk_self (id INTEGER PRIMARY KEY, parent_id INTEGER, FOREIGN KEY (parent_id) REFERENCES saya_fk_self(id))",
    ] {
        sqlx::query(stmt).execute(&setup).await.unwrap();
    }

    let options = sqlx::postgres::PgConnectOptions::from_str(&url).unwrap();
    let connector = PostgresConnector::from_options(options, ConnectorOptions::default());
    connector.connect().await.unwrap();
    let schema = connector.schema().await.unwrap();

    let find = |table: &str| {
        schema.databases[0]
            .schemas
            .iter()
            .flat_map(|schema| schema.tables.iter())
            .find(|t| t.name == table)
            .unwrap_or_else(|| panic!("{table} table missing"))
    };

    let child = find("saya_fk_child");
    assert_eq!(child.foreign_keys.len(), 1);
    let fk = &child.foreign_keys[0];
    assert_eq!(fk.columns, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(fk.referenced_table, "saya_fk_parent");
    assert_eq!(
        fk.referenced_columns,
        vec!["a".to_string(), "b".to_string()]
    );

    let self_ref = find("saya_fk_self");
    assert_eq!(self_ref.foreign_keys.len(), 1);
    let fk: &ForeignKey = &self_ref.foreign_keys[0];
    assert_eq!(fk.columns, vec!["parent_id".to_string()]);
    assert_eq!(fk.referenced_table, "saya_fk_self");

    for stmt in [
        "DROP TABLE saya_fk_child",
        "DROP TABLE saya_fk_parent",
        "DROP TABLE saya_fk_self",
    ] {
        sqlx::query(stmt).execute(&setup).await.unwrap();
    }
}
