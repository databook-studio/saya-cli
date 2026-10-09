//! `saya investigation` parameter surfaces (B1f) end-to-end over the real
//! binary and SQLite: `save`/`edit` declare `:name` parameters with
//! `--param-spec`, `run` binds them with `--param`, values never persist
//! anywhere on disk, and an engine without native binding refuses honestly
//! before connecting.

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
        "saya-investigation-params-{label}-{}",
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
/// over an empty database file.
fn harness_with_database(label: &str) -> Harness {
    let database = label_database(label);
    fs::write(&database, b"").unwrap();
    let mut h = harness(
        label,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    );
    h.database = database;
    h
}

fn label_database(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-params-db-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.join("data.sqlite3")
}

impl Harness {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_saya"))
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
            .env("HOME", self.root.join("home"))
            .output()
            .unwrap()
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

    /// Saves one parameterized investigation and returns its id.
    fn saved_id(&self, label: &str, sql: &str, specs: &[&str]) -> String {
        self.saved_id_for(label, sql, specs, None)
    }

    /// Saves with an explicit `--connection` (None = the default profile).
    fn saved_id_for(
        &self,
        label: &str,
        sql: &str,
        specs: &[&str],
        connection: Option<&str>,
    ) -> String {
        let mut args = vec!["investigation", "save", "--name", label, "--sql", sql];
        for spec in specs {
            args.push("--param-spec");
            args.push(spec);
        }
        if let Some(connection) = connection {
            args.push("--connection");
            args.push(connection);
        }
        let save = self.run(&args);
        assert_eq!(
            save.status.code(),
            Some(0),
            "save failed: {}{}",
            self.stdout(&save),
            self.stderr(&save)
        );
        self.document_ids().into_iter().next().unwrap()
    }
}

/// Seeds the `events` table with two rows through sqlx (direct, not through
/// saya).
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

// ---------------------------------------------------------------------------
// invariant 2: the value gates fire before any connection work
// ---------------------------------------------------------------------------

include!("investigation_params/preconnect_refusals.rs");

// ---------------------------------------------------------------------------
// invariant 2: capability honesty
// ---------------------------------------------------------------------------

include!("investigation_params/native_binding.rs");

// ---------------------------------------------------------------------------
// invariant 1: save and edit declare; the placeholders must match exactly
// ---------------------------------------------------------------------------

include!("investigation_params/spec_lifecycle.rs");

// ---------------------------------------------------------------------------
// invariant 3: values never persist
// ---------------------------------------------------------------------------

include!("investigation_params/persisted_values.rs");

include!("investigation_params/binding_edges.rs");
