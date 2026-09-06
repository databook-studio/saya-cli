use saya_connectors::{ConnectorOptions, DatabaseConnector, SqliteConnector};
use saya_types::{ConnectionError, ForeignKey, QueryRequest};
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

    // 5. cancel() is supported: SQLite is interruptible through the progress
    // handler, so asking a connector with nothing running to cancel succeeds
    // and leaves it usable.
    assert!(connector.cancel().await.is_ok());

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
        ..Default::default()
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

/// SQLite ships `sqrt`, `pow`, `ceil`, `floor`, `mod`, the logarithms and the
/// trigonometric functions, but only when it is compiled with
/// `SQLITE_ENABLE_MATH_FUNCTIONS` — the bundled build does not enable it by
/// default. Without them any question involving a distance, a rate or a
/// rounding boundary fails against a SQLite profile while working against a
/// server engine, which is a difference the user never asked for.
#[tokio::test]
async fn sqlite_has_the_standard_maths_functions() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("maths.db");
    create_test_fixture(&path).await;

    let connector = SqliteConnector::open(&path, true, ConnectorOptions::default())
        .await
        .expect("fixture opens");
    connector.connect().await.expect("connect");

    for expression in [
        "sqrt(4.0)",
        "pow(2.0, 3.0)",
        "ceil(1.2)",
        "floor(1.8)",
        "mod(5, 2)",
        "exp(0.0)",
        "ln(1.0)",
        "log10(100.0)",
        "sin(0.0)",
        "cos(0.0)",
        "acos(1.0)",
        "asin(0.0)",
        "atan(0.0)",
        "atan2(0.0, 1.0)",
        "radians(180.0)",
        "degrees(0.0)",
        "pi()",
    ] {
        let req = QueryRequest {
            sql: format!("SELECT {expression} AS value"),
            max_rows: 1,
        };
        let result = connector.execute(req).await;
        assert!(
            result.is_ok(),
            "`{expression}` must be available to a SQLite profile, got {:?}",
            result.err()
        );
    }
}

/// A query can fail because the SQL names something that is not there, and the
/// agent writing that SQL is the one who has to fix it. Reporting only "SQLite
/// query failed" leaves it guessing: a missing function, a misspelt table and a
/// syntax error are indistinguishable, so it retries blind and burns its turn
/// budget probing.
///
/// The identifiers echoed here come from the SQL the caller just wrote, not
/// from any row, so naming them discloses nothing the caller did not already
/// have. Anything the classifier does not recognise stays redacted.
#[tokio::test]
async fn a_failing_query_says_what_the_sql_got_wrong() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("errors.db");
    create_test_fixture(&path).await;

    let connector = SqliteConnector::open(&path, true, ConnectorOptions::default())
        .await
        .expect("fixture opens");
    connector.connect().await.expect("connect");

    for (sql, expected) in [
        ("SELECT no_such_fn(1) AS v", "no_such_fn"),
        ("SELECT * FROM not_a_table", "not_a_table"),
        ("SELECT not_a_column FROM t", "not_a_column"),
    ] {
        let req = QueryRequest {
            sql: sql.to_string(),
            max_rows: 1,
        };
        let err = connector
            .execute(req)
            .await
            .expect_err("the query must fail");
        let text = err.to_string();
        assert!(
            text.contains(expected),
            "the error must name `{expected}` so the caller can correct the SQL, got: {text}"
        );
    }
}

/// The classifier is an allowlist, not a pass-through: a failure it does not
/// recognise keeps the redacted wording, so a driver message that might carry
/// row values cannot reach the user through this path.
#[tokio::test]
async fn an_unrecognised_failure_stays_redacted() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("redacted.db");
    create_test_fixture(&path).await;

    let connector = SqliteConnector::open(&path, true, ConnectorOptions::default())
        .await
        .expect("fixture opens");
    connector.connect().await.expect("connect");

    // A write against a read-only connection: refused for a reason that is not
    // about a name in the SQL, so nothing is echoed back.
    let req = QueryRequest {
        sql: "INSERT INTO t (id) VALUES (99)".to_string(),
        max_rows: 1,
    };
    let err = connector
        .execute(req)
        .await
        .expect_err("a write must fail on a read-only connection");
    let text = err.to_string();
    assert!(
        !text.contains("99"),
        "an unrecognised failure must not echo query content: {text}"
    );
}

/// Builds the join graph the model otherwise has to guess at: a single-column
/// reference, a composite reference, a self-reference, and a table with none.
/// Every constraint must come back with its columns in declaration order,
/// because positional pairing is the only thing distinguishing a composite
/// `(a, b) -> (x, y)` from `(a, b) -> (y, x)`.
async fn create_fk_fixture(path: &Path) {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await.unwrap();

    sqlx::query(
        "CREATE TABLE customers (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL
        );",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TABLE orders (
            id INTEGER PRIMARY KEY,
            customer_id INTEGER NOT NULL,
            placed_on TEXT,
            FOREIGN KEY (customer_id) REFERENCES customers(id)
        );",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TABLE composite_parent (
            a INTEGER,
            b INTEGER,
            PRIMARY KEY (a, b)
        );",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TABLE composite_child (
            id INTEGER PRIMARY KEY,
            a INTEGER,
            b INTEGER,
            FOREIGN KEY (a, b) REFERENCES composite_parent(a, b)
        );",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TABLE nodes (
            id INTEGER PRIMARY KEY,
            parent_id INTEGER,
            label TEXT,
            FOREIGN KEY (parent_id) REFERENCES nodes(id)
        );",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("CREATE TABLE standalone (id INTEGER, note TEXT);")
        .execute(&pool)
        .await
        .unwrap();

    pool.close().await;
}

fn assert_fk(
    actual: &[ForeignKey],
    columns: &[&str],
    referenced_table: &str,
    referenced_columns: &[&str],
) {
    let matched = actual
        .iter()
        .find(|fk| fk.columns == columns)
        .unwrap_or_else(|| panic!("no foreign key with local columns {columns:?}; got {actual:?}"));
    assert_eq!(
        matched.referenced_schema, None,
        "SQLite foreign keys resolve within the same database"
    );
    assert_eq!(matched.referenced_table, referenced_table);
    assert_eq!(matched.referenced_columns, referenced_columns);
}

#[tokio::test]
async fn sqlite_discovers_foreign_keys_and_primary_keys() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("fk.db");
    create_fk_fixture(&path).await;

    let connector = SqliteConnector::open(&path, true, ConnectorOptions::default())
        .await
        .expect("fixture opens");
    connector.connect().await.expect("connect");

    let schema = connector.schema().await.expect("schema");
    let main = &schema.databases[0].schemas[0];
    let table = |name: &str| {
        main.tables
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} table missing"))
    };

    // Single-column reference.
    let orders = table("orders");
    assert_eq!(orders.primary_key, vec!["id".to_string()]);
    assert_eq!(orders.foreign_keys.len(), 1);
    assert_fk(&orders.foreign_keys, &["customer_id"], "customers", &["id"]);

    // Composite reference: columns must keep their declaration order.
    let composite_child = table("composite_child");
    assert_eq!(composite_child.primary_key, vec!["id".to_string()]);
    assert_eq!(composite_child.foreign_keys.len(), 1);
    assert_fk(
        &composite_child.foreign_keys,
        &["a", "b"],
        "composite_parent",
        &["a", "b"],
    );
    let composite_parent = table("composite_parent");
    assert_eq!(
        composite_parent.primary_key,
        vec!["a".to_string(), "b".to_string()]
    );

    // Self-reference resolves to the same table name.
    let nodes = table("nodes");
    assert_eq!(nodes.primary_key, vec!["id".to_string()]);
    assert_eq!(nodes.foreign_keys.len(), 1);
    assert_fk(&nodes.foreign_keys, &["parent_id"], "nodes", &["id"]);

    // A table with no constraints produces neither a primary key nor any FK.
    let customers = table("customers");
    assert_eq!(customers.primary_key, vec!["id".to_string()]);
    assert!(customers.foreign_keys.is_empty());

    let standalone = table("standalone");
    assert!(standalone.primary_key.is_empty());
    assert!(standalone.foreign_keys.is_empty());

    drop(connector);
    drop(temp_dir);
}
