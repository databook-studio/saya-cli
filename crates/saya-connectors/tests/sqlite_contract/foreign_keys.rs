use super::*;

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
