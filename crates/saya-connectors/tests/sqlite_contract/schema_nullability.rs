use super::*;

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
