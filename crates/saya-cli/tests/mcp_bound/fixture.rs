struct Fixture {
    root: PathBuf,
    config: PathBuf,
    connections: PathBuf,
    state: PathBuf,
    investigations: PathBuf,
    database: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let root =
        std::env::temp_dir().join(format!("saya-cli-mcp-bound-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("data.sqlite3");
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();
    let config = root.join("config.toml");
    std::fs::write(&config, "default_profile = 'local'\n").unwrap();
    let state = root.join("state.sqlite3");
    let investigations = root.join("investigations");
    std::fs::create_dir_all(&investigations).unwrap();
    std::fs::write(&database, b"").unwrap();
    Fixture {
        root,
        config,
        connections,
        state,
        investigations,
        database,
    }
}

impl Fixture {
    /// Seeds the `blobs` table through sqlx, directly (the connector is
    /// read-only; only the test writes).
    fn seed_blobs(&self) {
        self.seed_blobs_count(ROWS);
    }

    /// The same, with an explicit row count: the engine-truncation case needs
    /// more rows than the connector's own 16 MiB result cap will return.
    fn seed_blobs_count(&self, rows: u64) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let options = sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&self.database)
                    .create_if_missing(true);
                let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
                sqlx::query("CREATE TABLE blobs (id INTEGER PRIMARY KEY, payload TEXT NOT NULL)")
                    .execute(&pool)
                    .await
                    .unwrap();
                for id in 1..=rows {
                    sqlx::query("INSERT INTO blobs (id, payload) VALUES (?, ?)")
                        .bind(id as i64)
                        .bind("x".repeat(CELL))
                        .execute(&pool)
                        .await
                        .unwrap();
                }
                pool.close().await;
            });
    }
}
