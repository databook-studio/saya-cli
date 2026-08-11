use saya_connectors::{ConnectorOptions, DatabaseConnector, SqliteConnector};
use saya_types::{ConnectionError, QueryRequest};
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

#[tokio::test]
async fn test_sqlite_missing_path_fails_eagerly_and_does_not_create_file() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("saya_sqlite_nonexistent_12345.db");

    let opts = ConnectorOptions::default();
    let res = SqliteConnector::open(&path, true, opts).await;
    assert!(res.is_err(), "Opening missing file must fail eagerly");
    assert!(!path.exists(), "Missing file must NOT be created on disk");

    if let Err(err) = res {
        let err_msg = err.to_string();
        assert!(
            !err_msg.contains("saya_sqlite_nonexistent_12345.db"),
            "Error message leaked path: {err_msg}"
        );
    }
}

#[tokio::test]
async fn test_sqlite_in_memory_rejected() {
    let opts = ConnectorOptions::default();
    let res = SqliteConnector::open(std::path::Path::new(":memory:"), true, opts).await;
    assert!(matches!(res, Err(ConnectionError::InvalidConfiguration(_))));
}

#[tokio::test]
async fn test_sqlite_contract_full() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("full.db");
    create_test_fixture(&path).await;

    let opts = ConnectorOptions::default();
    let connector = SqliteConnector::open(&path, true, opts)
        .await
        .expect("Opening existing fixture db should succeed");

    // 1. connect()
    connector.connect().await.expect("connect() should succeed");

    // 2. schema()
    let schema_tree = connector.schema().await.expect("schema() should succeed");
    assert_eq!(schema_tree.databases.len(), 1);
    let db = &schema_tree.databases[0];
    assert_eq!(db.name, "full");
    assert_eq!(db.schemas.len(), 1);
    let main_schema = &db.schemas[0];
    assert_eq!(main_schema.name, "main");

    let table_names: Vec<&str> = main_schema.tables.iter().map(|t| t.name.as_str()).collect();
    assert!(table_names.contains(&"t"), "Table t missing in schema");
    assert!(table_names.contains(&"v"), "View v missing in schema");
    assert!(
        table_names.contains(&r#"weird "name""#),
        "Hostile table weird \"name\" missing in schema"
    );

    let t_table = main_schema.tables.iter().find(|t| t.name == "t").unwrap();
    let col_names: Vec<&str> = t_table.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        col_names,
        vec![
            "id", "amount", "label", "blob_col", "maybe", "gen", "no_type"
        ]
    );

    // Verify column properties
    let gen_col = t_table.columns.iter().find(|c| c.name == "gen").unwrap();
    assert_eq!(gen_col.data_type.to_uppercase(), "INTEGER");

    let no_type_col = t_table
        .columns
        .iter()
        .find(|c| c.name == "no_type")
        .unwrap();
    assert_eq!(no_type_col.data_type, "");

    // 3. execute() SELECT with max_rows = 1 (forces truncation since fixture has 2 rows)
    let req = QueryRequest {
        sql: "SELECT id, amount, label, blob_col, maybe FROM t ORDER BY id".to_string(),
        max_rows: 1,
    };
    let res = connector
        .execute(req)
        .await
        .expect("execute() should succeed");
    assert_eq!(
        res.executed_sql,
        "SELECT id, amount, label, blob_col, maybe FROM t ORDER BY id"
    );
    assert_eq!(
        res.columns,
        vec!["id", "amount", "label", "blob_col", "maybe"]
    );
    assert_eq!(res.row_count, 1);
    assert!(
        res.truncated,
        "Result should be truncated when max_rows < row count"
    );

    let first_row = match &res.rows[0] {
        Value::Array(arr) => arr,
        other => panic!("Row is not JSON Array: {other:?}"),
    };
    assert_eq!(first_row[0], Value::from(1));
    assert_eq!(first_row[1], Value::from(3.5));
    assert_eq!(first_row[2], Value::String("hello".to_string()));
    assert_eq!(first_row[3], Value::String("00ff10".to_string()));
    assert_eq!(first_row[4], Value::Null);

    // 4. execute() write statement rejected by safety layer
    let mut_req = QueryRequest {
        sql: "UPDATE t SET amount = 0.0 WHERE id = 1".to_string(),
        max_rows: 10,
    };
    assert!(
        connector.execute(mut_req).await.is_err(),
        "Mutating query must be rejected by safety policy"
    );

    // 5. cancel() returns Unsupported
    assert!(matches!(
        connector.cancel().await,
        Err(ConnectionError::Unsupported(_))
    ));

    drop(connector);
    drop(temp_dir);
}

#[tokio::test]
async fn test_sqlite_driver_read_only_pragma() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("pragma_read_only.db");
    create_test_fixture(&path).await;

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .read_only(true)
        .pragma("query_only", "ON");
    let pool = SqlitePool::connect_with(options).await.unwrap();

    let write_res = sqlx::query("UPDATE t SET amount = 1.0 WHERE id = 1")
        .execute(&pool)
        .await;
    assert!(
        write_res.is_err(),
        "Driver-level read_only / query_only PRAGMA must reject writes"
    );

    pool.close().await;
    drop(temp_dir);
}

#[tokio::test]
async fn test_sqlite_query_timeout_interrupts_and_cleans_up_connection() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("timeout_test.db");
    create_test_fixture(&path).await;

    let opts = ConnectorOptions {
        query_timeout_seconds: 1,
        max_connections: 1,
    };
    let connector = SqliteConnector::open(&path, true, opts)
        .await
        .expect("Opening fixture db should succeed");

    let start = std::time::Instant::now();
    let infinite_req = QueryRequest {
        sql: "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM cnt) SELECT count(*) FROM cnt;".to_string(),
        max_rows: 10,
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

#[tokio::test]
async fn test_sqlite_schema_nullability_primary_keys() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("pk_nullability.db");

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await.unwrap();

    sqlx::query(
        "CREATE TABLE pk_demo (
            id INTEGER PRIMARY KEY,
            name TEXT,
            note TEXT NOT NULL
        );",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TABLE wr (
            k TEXT PRIMARY KEY,
            v INTEGER
        ) WITHOUT ROWID;",
    )
    .execute(&pool)
    .await
    .unwrap();

    pool.close().await;

    let opts = ConnectorOptions::default();
    let connector = SqliteConnector::open(&path, true, opts)
        .await
        .expect("Opening fixture db should succeed");

    let schema_tree = connector.schema().await.expect("schema() should succeed");
    let main_schema = &schema_tree.databases[0].schemas[0];

    // Assert pk_demo columns nullability
    let pk_demo = main_schema
        .tables
        .iter()
        .find(|t| t.name == "pk_demo")
        .expect("pk_demo table missing");

    let id_col = pk_demo.columns.iter().find(|c| c.name == "id").unwrap();
    let name_col = pk_demo.columns.iter().find(|c| c.name == "name").unwrap();
    let note_col = pk_demo.columns.iter().find(|c| c.name == "note").unwrap();

    assert!(
        !id_col.nullable,
        "INTEGER PRIMARY KEY `id` should be non-nullable"
    );
    assert!(name_col.nullable, "`name` should be nullable");
    assert!(!note_col.nullable, "`note` NOT NULL should be non-nullable");

    // Assert wr columns nullability
    let wr = main_schema
        .tables
        .iter()
        .find(|t| t.name == "wr")
        .expect("wr table missing");

    let k_col = wr.columns.iter().find(|c| c.name == "k").unwrap();
    let v_col = wr.columns.iter().find(|c| c.name == "v").unwrap();

    assert!(
        !k_col.nullable,
        "WITHOUT ROWID PK `k` should be non-nullable"
    );
    assert!(v_col.nullable, "`v` should be nullable");

    drop(connector);
    drop(temp_dir);
}

#[tokio::test]
async fn test_sqlite_schema_nullability_composite_pk() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("composite_pk.db");

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await.unwrap();

    sqlx::query("CREATE TABLE comp (a INTEGER, b INTEGER, PRIMARY KEY (a, b));")
        .execute(&pool)
        .await
        .unwrap();

    pool.close().await;

    let opts = ConnectorOptions::default();
    let connector = SqliteConnector::open(&path, true, opts)
        .await
        .expect("Opening fixture db should succeed");

    let schema_tree = connector.schema().await.expect("schema() should succeed");
    let main_schema = &schema_tree.databases[0].schemas[0];

    let comp = main_schema
        .tables
        .iter()
        .find(|t| t.name == "comp")
        .expect("comp table missing");

    let a_col = comp.columns.iter().find(|c| c.name == "a").unwrap();
    let b_col = comp.columns.iter().find(|c| c.name == "b").unwrap();

    assert!(a_col.nullable, "Composite PK column `a` should be nullable");
    assert!(b_col.nullable, "Composite PK column `b` should be nullable");

    drop(connector);
    drop(temp_dir);
}

#[tokio::test]
async fn test_sqlite_schema_nullability_desc_pk() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("desc_pk.db");

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await.unwrap();

    sqlx::query("CREATE TABLE d (id INTEGER PRIMARY KEY DESC, v TEXT);")
        .execute(&pool)
        .await
        .unwrap();

    pool.close().await;

    let opts = ConnectorOptions::default();
    let connector = SqliteConnector::open(&path, true, opts)
        .await
        .expect("Opening fixture db should succeed");

    let schema_tree = connector.schema().await.expect("schema() should succeed");
    let main_schema = &schema_tree.databases[0].schemas[0];

    let d_table = main_schema
        .tables
        .iter()
        .find(|t| t.name == "d")
        .expect("d table missing");

    let id_col = d_table.columns.iter().find(|c| c.name == "id").unwrap();
    let v_col = d_table.columns.iter().find(|c| c.name == "v").unwrap();

    assert!(
        id_col.nullable,
        "INTEGER PRIMARY KEY DESC `id` should be nullable"
    );
    assert!(v_col.nullable, "`v` should be nullable");

    drop(connector);
    drop(temp_dir);
}

#[tokio::test]
async fn test_sqlite_schema_nullability_ddl_false_positive() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("ddl_false_positive.db");

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await.unwrap();

    sqlx::query("CREATE TABLE fake (id INTEGER PRIMARY KEY, note TEXT DEFAULT 'WITHOUT ROWID');")
        .execute(&pool)
        .await
        .unwrap();

    pool.close().await;

    let opts = ConnectorOptions::default();
    let connector = SqliteConnector::open(&path, true, opts)
        .await
        .expect("Opening fixture db should succeed");

    let schema_tree = connector.schema().await.expect("schema() should succeed");
    let main_schema = &schema_tree.databases[0].schemas[0];

    let fake_table = main_schema
        .tables
        .iter()
        .find(|t| t.name == "fake")
        .expect("fake table missing");

    let id_col = fake_table.columns.iter().find(|c| c.name == "id").unwrap();
    let note_col = fake_table
        .columns
        .iter()
        .find(|c| c.name == "note")
        .unwrap();

    assert!(
        !id_col.nullable,
        "`id` in table with default 'WITHOUT ROWID' should be non-nullable"
    );
    assert!(note_col.nullable, "`note` should be nullable");

    drop(connector);
    drop(temp_dir);
}
