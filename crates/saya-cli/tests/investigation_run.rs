//! `saya investigation run <id>` (S7) end-to-end: replays a saved
//! investigation through the real binary over a temp SQLite database, an
//! isolated `SAYA_INVESTIGATIONS_DIR`, `SAYA_CONFIG_HOME`, `SAYA_STATE_DB`,
//! and `HOME`. Replay takes an explicit target only — never the active or
//! default profile, never an AI provider — and a stale review is refused
//! until `--revalidate` (D4).

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use sqlx::SqlitePool;

struct Harness {
    root: PathBuf,
    connections: PathBuf,
    config: PathBuf,
    investigations: PathBuf,
    state: PathBuf,
    database: PathBuf,
}

/// A temp root whose connections file is given verbatim; config, state, and
/// home dirs are isolated beneath it.
fn harness(label: &str, connections_toml: String) -> Harness {
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-run-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let connections = root.join("connections.toml");
    fs::write(&connections, connections_toml).unwrap();
    let config = root.join("config.toml");
    fs::write(
        &config,
        "default_profile = 'local'\n\n[run]\nmax_rows = 50\n",
    )
    .unwrap();
    for dir in ["investigations", "config-home", "home"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    Harness {
        investigations: root.join("investigations"),
        connections,
        config,
        state: root.join("state.sqlite3"),
        database: root.join("data.sqlite3"),
        root,
    }
}

/// One sqlite profile named `local`, auto-selected as the active profile,
/// over an empty database file (replay itself creates the tables it needs).
fn harness_with_database(label: &str) -> Harness {
    let mut h = harness(
        label,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            label_database(label).display()
        ),
    );
    h.database = label_database(label);
    fs::write(&h.database, b"").unwrap();
    h
}

fn label_database(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-db-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.join("data.sqlite3")
}

impl Harness {
    fn run(&self, args: &[&str]) -> Output {
        self.run_env(args, &[])
    }

    /// Runs the binary with extra environment variables layered on top (used
    /// to configure a provider that points at an unroutable address).
    fn run_env(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
        command
            .args([
                "--non-interactive",
                "--format",
                "json",
                "--config",
                self.config.to_str().unwrap(),
                "--connections",
                self.connections.to_str().unwrap(),
            ])
            .args(args)
            .env("SAYA_STATE_DB", &self.state)
            .env("SAYA_INVESTIGATIONS_DIR", &self.investigations)
            .env("SAYA_CONFIG_HOME", self.root.join("config-home"))
            .env("HOME", self.root.join("home"));
        for (key, value) in envs {
            command.env(key, value);
        }
        command.output().unwrap()
    }

    fn stdout(&self, output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn stderr(&self, output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    /// The ids of the documents on disk, sorted (the stems name them).
    fn document_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.investigations) {
            for entry in entries.filter_map(Result::ok) {
                if entry.path().extension().is_some_and(|ext| ext == "json") {
                    ids.push(
                        entry
                            .path()
                            .file_stem()
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
        }
        ids.sort();
        ids
    }

    fn document_path(&self, id: &str) -> PathBuf {
        self.investigations.join(format!("{id}.json"))
    }

    fn binding_path(&self, id: &str) -> PathBuf {
        self.investigations.join("local").join(format!("{id}.json"))
    }
}

/// Seeds the `events` table with two rows through sqlx (direct, not through
/// saya), the way the schema-change test later alters it.
async fn seed_events(database: &Path) {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("CREATE TABLE events (id INTEGER PRIMARY KEY, label TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO events (id, label) VALUES (1, 'first'), (2, 'second')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

async fn event_rows(database: &Path) -> i64 {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options).await.unwrap();
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    count
}

/// Saves one investigation through the binary and returns its id. The
/// profile is passed explicitly because a multi-profile connections file
/// auto-selects nothing.
fn saved_id(h: &Harness, connection: Option<&str>) -> String {
    let mut args = vec![
        "investigation",
        "save",
        "--name",
        "Order events",
        "--description",
        "events in order",
        "--sql",
        "SELECT id, label FROM events ORDER BY id",
    ];
    if let Some(connection) = connection {
        args.push("--connection");
        args.push(connection);
    }
    let save = h.run(&args);
    assert_eq!(
        save.status.code(),
        Some(0),
        "save failed: {}{}",
        h.stdout(&save),
        h.stderr(&save)
    );
    h.document_ids()
        .into_iter()
        .next()
        .expect("exactly one document")
}

/// Saves one investigation with an exact SQL string and returns its id.
fn saved_sql_id(h: &Harness, sql: &str) -> String {
    let save = h.run(&[
        "investigation",
        "save",
        "--name",
        "Order events",
        "--sql",
        sql,
    ]);
    assert_eq!(
        save.status.code(),
        Some(0),
        "save failed: {}{}",
        h.stdout(&save),
        h.stderr(&save)
    );
    h.document_ids()
        .into_iter()
        .next()
        .expect("exactly one document")
}

include!("investigation_run/schema_support.rs");
include!("investigation_run/provider_config.rs");
include!("investigation_run/replay_preflight.rs");

include!("investigation_run/stored_objects.rs");
include!("investigation_run/schema_review.rs");

include!("investigation_run/read_only_mapping.rs");

include!("investigation_run/report_publication.rs");

include!("investigation_run/fixture_identity.rs");
