use super::*;

#[tokio::test]
async fn test_sqlite_byte_budget_truncates_large_cell() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("large_cell.db");

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await.unwrap();

    sqlx::query("CREATE TABLE big_data (id INTEGER PRIMARY KEY, content TEXT);")
        .execute(&pool)
        .await
        .unwrap();

    let large_string = "a".repeat(1_500_000);
    sqlx::query("INSERT INTO big_data (id, content) VALUES (1, ?);")
        .bind(&large_string)
        .execute(&pool)
        .await
        .unwrap();

    pool.close().await;

    let opts = ConnectorOptions::default();
    let connector = SqliteConnector::open(&path, true, opts)
        .await
        .expect("Opening fixture db should succeed");

    let req = QueryRequest {
        sql: "SELECT id, content FROM big_data".to_string(),
        max_rows: 10,
        params: Vec::new(),
    };
    let res = connector
        .execute(req)
        .await
        .expect("execute() should succeed");

    assert_eq!(res.row_count, 1);
    assert!(
        !res.truncated,
        "Single row under MAX_RESULT_BYTES should not truncate query"
    );

    let row = match &res.rows[0] {
        Value::Array(arr) => arr,
        other => panic!("Row is not JSON Array: {other:?}"),
    };

    let content_val = match &row[1] {
        Value::String(s) => s,
        other => panic!("Content is not JSON String: {other:?}"),
    };

    assert!(
        content_val.len() < large_string.len(),
        "Cell string should be truncated"
    );
    assert!(
        content_val.contains("…[truncated "),
        "Truncated cell must contain truncation marker"
    );
    assert!(
        content_val.len() <= 1_048_576 + 50,
        "Cell size must be capped near MAX_CELL_BYTES + marker length, got {}",
        content_val.len()
    );

    drop(connector);
    drop(temp_dir);
}

#[tokio::test]
async fn test_sqlite_result_level_byte_budget_end_to_end() {
    // Note: MAX_RESULT_BYTES is pub(crate) in common.rs (16 MiB = 16,777,216 bytes)
    // and is not exported in the public API of saya_connectors. We define a local constant
    // for asserting byte budget bounds in this integration test.
    const MAX_RESULT_BYTES_LOCAL: usize = 16 * 1024 * 1024;

    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("result_byte_budget_e2e.db");

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await.unwrap();

    sqlx::query("CREATE TABLE big_result (id INTEGER PRIMARY KEY, payload TEXT);")
        .execute(&pool)
        .await
        .unwrap();

    // 35 rows * 500_000 bytes = 17,500,000 bytes (~16.69 MiB > 16 MiB budget)
    let payload_str = "p".repeat(500_000);
    let mut tx = pool.begin().await.unwrap();
    for i in 1..=35 {
        sqlx::query("INSERT INTO big_result (id, payload) VALUES (?, ?);")
            .bind(i)
            .bind(&payload_str)
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();

    pool.close().await;

    let opts = ConnectorOptions::default();
    let connector = SqliteConnector::open(&path, true, opts)
        .await
        .expect("Opening fixture db should succeed");

    let req = QueryRequest {
        sql: "SELECT id, payload FROM big_result ORDER BY id".to_string(),
        max_rows: 100,
        params: Vec::new(),
    };
    let res = connector
        .execute(req)
        .await
        .expect("execute() should succeed");

    assert!(
        res.truncated,
        "QueryResult must be marked truncated when total bytes exceed MAX_RESULT_BYTES"
    );
    assert!(
        res.row_count < 35,
        "Row count ({}) must be less than total inserted rows (35) due to byte budget cap",
        res.row_count
    );

    let total_bytes: usize = res
        .rows
        .iter()
        .map(|row_val| match row_val {
            Value::Array(cells) => cells
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.len(),
                    _ => 8,
                })
                .sum::<usize>(),
            _ => 0,
        })
        .sum();

    assert!(
        total_bytes > MAX_RESULT_BYTES_LOCAL,
        "Total result bytes ({total_bytes}) must cross MAX_RESULT_BYTES threshold ({MAX_RESULT_BYTES_LOCAL})"
    );
    let one_row_approx = 500_000 + 8;
    assert!(
        total_bytes <= MAX_RESULT_BYTES_LOCAL + one_row_approx,
        "Total byte size ({total_bytes}) must be bounded by MAX_RESULT_BYTES + one row ({})",
        MAX_RESULT_BYTES_LOCAL + one_row_approx
    );

    drop(connector);
    drop(temp_dir);
}
