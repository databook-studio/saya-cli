/// A connector that never touches a live database: the probe tool calls run
/// against it, and no test here reads the store.
struct IdleConnector;

#[async_trait]
impl saya_connectors::DatabaseConnector for IdleConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree {
            databases: vec![Database {
                name: "catalog".into(),
                schemas: vec![Schema {
                    name: "public".into(),
                    tables: vec![orders_table()],
                }],
            }],
        })
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(req.sql))
    }
}

fn orders_table() -> Table {
    Table {
        name: "orders".into(),
        columns: vec![
            Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            },
            Column {
                name: "created_at".into(),
                data_type: "timestamp".into(),
                nullable: false,
            },
        ],
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

fn registry_for(name: &str) -> ConnectionRegistry {
    let identity: ProfileIdentity = crate::profile_identity::profile_identity(
        name,
        &DatabaseProfile::DuckDb {
            path: "runtime-budget.duckdb".into(),
            read_only: Some(true),
        },
        Path::new("/runtime-budget-test/connections.toml"),
    );
    let mut registry = ConnectionRegistry::new(name);
    registry.insert(
        name,
        ConnectionEntry {
            connector: Box::new(IdleConnector),
            dialect: SqlDialect::DuckDb,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    registry
}

async fn sqlite_registry(root: &Path) -> ConnectionRegistry {
    let database = root.join("runtime-budget.sqlite3");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .expect("seed SQLite runtime budget database");
    sqlx::query("CREATE TABLE orders (id INTEGER)")
        .execute(&seed)
        .await
        .expect("create SQLite budget table");
    sqlx::query("INSERT INTO orders (id) VALUES (1)")
        .execute(&seed)
        .await
        .expect("seed SQLite budget row");
    seed.close().await;
    let connector = SqliteConnector::open(&database, true, ConnectorOptions::default())
        .await
        .expect("open production SQLite connector");
    let mut registry = ConnectionRegistry::new("analytics");
    registry.insert(
        "analytics",
        ConnectionEntry {
            connector: Box::new(connector),
            dialect: SqlDialect::Sqlite,
            profile_id: Some("runtime-budget-sqlite".into()),
        },
    );
    registry
}
