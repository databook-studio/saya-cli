use crate::demo::{data, fixture};
use sqlx::SqlitePool;

pub(crate) const SCHEMA: [&str; 4] = [
    "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT, \
     region TEXT NOT NULL, signup_date TEXT NOT NULL, status TEXT NOT NULL)",
    "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL \
     REFERENCES customers(id), order_date TEXT NOT NULL, amount_cents INTEGER, \
     status TEXT NOT NULL)",
    "CREATE TABLE customer_contacts (customer_id INTEGER NOT NULL \
     REFERENCES customers(id), channel TEXT NOT NULL, value TEXT NOT NULL)",
    "CREATE TABLE saya_demo_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
];

pub(crate) async fn create_fixture_db(path: &std::path::Path) -> Result<(), String> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options)
        .await
        .map_err(|error| format!("create demo database: {error}"))?;
    let written = write_schema_and_rows(&pool).await;
    pool.close().await;
    written
}

async fn write_schema_and_rows(pool: &SqlitePool) -> Result<(), String> {
    for statement in SCHEMA {
        sqlx::query(statement)
            .execute(pool)
            .await
            .map_err(write_error)?;
    }
    let demo = data::build();
    for row in &demo.customers {
        sqlx::query(
            "INSERT INTO customers (id, name, email, region, signup_date, status) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(row.id)
        .bind(&row.name)
        .bind(&row.email)
        .bind(&row.region)
        .bind(&row.signup_date)
        .bind(&row.status)
        .execute(pool)
        .await
        .map_err(write_error)?;
    }
    for row in &demo.orders {
        sqlx::query(
            "INSERT INTO orders (id, customer_id, order_date, amount_cents, status) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(row.id)
        .bind(row.customer_id)
        .bind(&row.order_date)
        .bind(row.amount_cents)
        .bind(&row.status)
        .execute(pool)
        .await
        .map_err(write_error)?;
    }
    for row in &demo.contacts {
        sqlx::query("INSERT INTO customer_contacts (customer_id, channel, value) VALUES (?, ?, ?)")
            .bind(row.customer_id)
            .bind(&row.channel)
            .bind(&row.value)
            .execute(pool)
            .await
            .map_err(write_error)?;
    }
    sqlx::query("INSERT INTO saya_demo_meta (key, value) VALUES ('fixture_version', ?)")
        .bind(fixture::FIXTURE_VERSION)
        .execute(pool)
        .await
        .map_err(write_error)?;
    Ok(())
}

fn write_error(error: sqlx::Error) -> String {
    format!("write demo fixture: {error}")
}
