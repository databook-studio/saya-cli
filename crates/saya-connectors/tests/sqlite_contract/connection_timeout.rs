use super::*;

#[tokio::test]
async fn test_sqlite_query_timeout_interrupts_and_cleans_up_connection() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("timeout_test.db");
    create_test_fixture(&path).await;

    let opts = ConnectorOptions {
        query_timeout_seconds: 1,
        max_connections: 1,
        ..Default::default()
    };
    let connector = SqliteConnector::open(&path, true, opts)
        .await
        .expect("Opening fixture db should succeed");

    let start = std::time::Instant::now();
    let infinite_req = QueryRequest {
        sql: "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM cnt) SELECT count(*) FROM cnt;".to_string(),
        max_rows: 10,
        params: Vec::new(),
    };

    let res = connector.execute(infinite_req).await;
    let elapsed = start.elapsed();

    assert!(
        res.is_err(),
        "Long-running query must be interrupted and return Err"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "Query timeout must trigger within a few seconds, took {:?}",
        elapsed
    );

    if let Err(err) = res {
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("timed out") || err_msg.contains("Query failed"),
            "Error message should indicate timeout/failure: {err_msg}"
        );
        assert!(
            !err_msg.contains(path.to_str().unwrap_or("")),
            "Error message must not leak file path: {err_msg}"
        );
    }

    let reuse_req = QueryRequest {
        sql: "SELECT 1".to_string(),
        max_rows: 10,
        params: Vec::new(),
    };
    let reuse_res = connector.execute(reuse_req).await;
    assert!(
        reuse_res.is_ok(),
        "Reusing connection after timeout must succeed (no pool poisoning)"
    );

    drop(connector);
    drop(temp_dir);
}

#[tokio::test]
async fn test_sqlite_finite_query_succeeds_under_normal_timeout() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("finite_timeout_test.db");
    create_test_fixture(&path).await;

    let opts = ConnectorOptions::default();
    let connector = SqliteConnector::open(&path, true, opts)
        .await
        .expect("Opening fixture db should succeed");

    let req = QueryRequest {
        sql: "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c WHERE x < 5000) SELECT count(*) AS n FROM c".to_string(),
        max_rows: 10,
        params: Vec::new(),
    };

    let res = connector
        .execute(req)
        .await
        .expect("Finite query exceeding 1000 VM instructions must succeed under normal timeout");

    assert_eq!(res.row_count, 1);
    assert!(!res.truncated);
    let row = match &res.rows[0] {
        Value::Array(arr) => arr,
        other => panic!("Row is not JSON Array: {other:?}"),
    };
    assert_eq!(row[0], Value::from(5000));

    drop(connector);
    drop(temp_dir);
}
