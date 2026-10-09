//! `saya investigation save|list|show|delete` (S6) end-to-end: runs the real
//! binary with an isolated `SAYA_INVESTIGATIONS_DIR`, `SAYA_CONFIG_HOME`,
//! `SAYA_STATE_DB`, and `HOME` over a temp SQLite profile. Saving validates
//! and never connects, so the profile's database is an empty file (mirrors
//! the isolation of `tests/sqlite_cli.rs`).

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

struct Harness {
    root: PathBuf,
    connections: PathBuf,
    config: PathBuf,
    investigations: PathBuf,
    state: PathBuf,
}

/// A temp root with a single-profile connections file (auto-selected as the
/// active profile), a starter config, and isolated home/config state dirs.
fn harness(label: &str, connections_toml: &str) -> Harness {
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-cli-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let connections = root.join("connections.toml");
    fs::write(&connections, connections_toml).unwrap();
    let config = root.join("config.toml");
    fs::write(&config, "[run]\nmax_rows = 1\n").unwrap();
    for dir in ["investigations", "config-home", "home"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    Harness {
        investigations: root.join("investigations"),
        connections,
        config,
        state: root.join("state.sqlite3"),
        root,
    }
}

fn harness_with_database(label: &str) -> Harness {
    let database = label_database(label);
    fs::write(&database, b"").unwrap();
    harness(
        label,
        &format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
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
}

/// Saves one investigation through the binary and returns its id.
fn saved_id(h: &Harness) -> String {
    let save = h.run(&[
        "investigation",
        "save",
        "--name",
        "Order events",
        "--description",
        "events in order",
        "--sql",
        "SELECT id, label FROM events ORDER BY id",
    ]);
    assert_eq!(
        save.status.code(),
        Some(0),
        "save failed: {}",
        h.stderr(&save)
    );
    h.document_ids()
        .into_iter()
        .next()
        .expect("exactly one document")
}

include!("investigation_cli/lifecycle.rs");

/// Two `cargo test` processes must never share a fixture directory: every
/// temp root this file builds carries this process's id.
#[test]
fn fixture_roots_carry_this_process_id() {
    let h = harness_with_database("isolation");
    let database_root = label_database("isolation").parent().unwrap().to_path_buf();
    let suffix = format!("-{}", std::process::id());
    assert!(
        h.root.to_str().unwrap().ends_with(&suffix),
        "the harness root must be per-process: {}",
        h.root.display()
    );
    assert!(
        database_root.to_str().unwrap().ends_with(&suffix),
        "the database fixture root must be per-process: {}",
        database_root.display()
    );
    let _ = fs::remove_dir_all(&h.root);
    let _ = fs::remove_dir_all(&database_root);
}

// -- edit (A2.5 reopened) --------------------------------------------------

include!("investigation_cli/edits.rs");
