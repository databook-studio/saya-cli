/// Runs one DDL statement through sqlx, the schema change a stale review
/// must catch.
async fn alter_table(database: &Path, sql: &str) {
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(database);
    let pool = SqlitePool::connect_with(options).await.unwrap();
    sqlx::query(sql).execute(&pool).await.unwrap();
    pool.close().await;
}
