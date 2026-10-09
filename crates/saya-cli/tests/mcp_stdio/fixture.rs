/// An isolated fixture: two sqlite profiles (`local` and `other`) over one
/// seeded database, config naming `local` the default, and isolated state
/// and investigations paths.
struct Fixture {
    root: PathBuf,
    config: PathBuf,
    connections: PathBuf,
    state: PathBuf,
    investigations: PathBuf,
    database: PathBuf,
}

fn crate_fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("saya-cli-mcp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("data.sqlite3");
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n\n\
             [profiles.other]\ntype = 'sqlite'\npath = '{}'\n",
            database.display(),
            root.join("other.sqlite3").display(),
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
    /// Seeds the `events` table through sqlx, directly (the connector is
    /// read-only; only the test writes).
    fn seed_events(&self) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let options = sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(&self.database)
                    .create_if_missing(true);
                let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
                sqlx::query("CREATE TABLE events (id INTEGER PRIMARY KEY, label TEXT NOT NULL)")
                    .execute(&pool)
                    .await
                    .unwrap();
                sqlx::query("INSERT INTO events (id, label) VALUES (1, 'first'), (2, 'second')")
                    .execute(&pool)
                    .await
                    .unwrap();
                pool.close().await;
            });
    }

    /// Seeds a sentinel table into `other.sqlite3`: if a replay ever reached
    /// this non-allowlisted database, an investigation reading it would
    /// succeed instead of being refused.
    fn seed_sentinel(&self, label: &str) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let options = sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(self.root.join("other.sqlite3"))
                    .create_if_missing(true);
                let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
                sqlx::query("CREATE TABLE sentinel (label TEXT NOT NULL)")
                    .execute(&pool)
                    .await
                    .unwrap();
                sqlx::query("INSERT INTO sentinel (label) VALUES (?)")
                    .bind(label)
                    .execute(&pool)
                    .await
                    .unwrap();
                pool.close().await;
            });
    }

    /// Runs a CLI subcommand against this fixture with the same isolated
    /// environment the server gets, for the seeding steps the MCP surface
    /// itself never offers.
    fn cli(&self, args: &[&str]) -> (i32, String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_saya"))
            .args([
                "--non-interactive",
                "--config",
                self.config.to_str().unwrap(),
                "--connections",
                self.connections.to_str().unwrap(),
            ])
            .args(args)
            .env("SAYA_STATE_DB", &self.state)
            .env("SAYA_INVESTIGATIONS_DIR", &self.investigations)
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("home").join(".config"))
            .env_remove("SAYA_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// Saves one investigation through the CLI and returns its id.
    fn save_investigation(&self, name: &str, sql: &str, connection: &str) -> String {
        let (code, out, err) = self.cli(&[
            "investigation",
            "save",
            "--name",
            name,
            "--sql",
            sql,
            "--connection",
            connection,
        ]);
        assert_eq!(code, 0, "save failed: {out}{err}");
        out.lines()
            .next()
            .expect("the id is the first line")
            .trim()
            .to_string()
    }
}
