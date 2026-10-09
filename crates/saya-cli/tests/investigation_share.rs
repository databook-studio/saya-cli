//! `saya investigation export|import` (S8, D5) end-to-end: runs the real
//! binary with two isolated roots (separate `SAYA_INVESTIGATIONS_DIR`,
//! `SAYA_CONFIG_HOME`, `SAYA_STATE_DB`, and `HOME`) so a definition moves
//! between "machines" as a file. Export writes only the portable document;
//! import validates the whole file, previews it, stores it with no local
//! binding, and never executes anything or connects to a database.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Harness {
    root: PathBuf,
    connections: PathBuf,
    config: PathBuf,
    investigations: PathBuf,
    state: PathBuf,
}

/// A temp root with a single-profile connections file, a starter config, and
/// isolated home/config state dirs. `connections_toml` is verbatim, so a test
/// can point the profile at a path that must never be created.
fn harness(label: &str, connections_toml: &str) -> Harness {
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-share-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("config-home")).unwrap();
    fs::create_dir_all(root.join("home")).unwrap();
    let connections = root.join("connections.toml");
    fs::write(&connections, connections_toml).unwrap();
    let config = root.join("config.toml");
    fs::write(&config, "[run]\nmax_rows = 1\n").unwrap();
    Harness {
        investigations: root.join("investigations"),
        connections,
        config,
        state: root.join("state.sqlite3"),
        root,
    }
}

/// A harness whose profile points at an existing (empty) SQLite file.
fn harness_with_database(label: &str) -> Harness {
    let database = root_database(label);
    fs::write(&database, b"").unwrap();
    harness(
        label,
        &format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
}

fn root_database(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-share-db-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root.join("data.sqlite3")
}

/// A hand-written, fully valid v1 definition, as an exporter on another
/// machine would have produced. `schema_fingerprint` is an explicit null so
/// the document never depends on serde's missing-Option default.
const DEFINITION_JSON: &str = r#"{
  "format": "saya.investigation",
  "version": 1,
  "id": "order-events-01234567",
  "revision": 1,
  "name": "Order events",
  "sql": "SELECT id, label FROM events ORDER BY id",
  "dialect": "sqlite",
  "connection": "local",
  "objects": ["events"],
  "schema_fingerprint": null,
  "created_unix_ms": 1700000000000,
  "updated_unix_ms": 1700000000000
}"#;

/// Writes `DEFINITION_JSON` (or a mutated variant) to a file and returns it.
fn definition_file(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).unwrap();
    path
}

impl Harness {
    fn run(&self, args: &[&str]) -> Output {
        self.run_args(&{
            let mut all = vec![
                "--non-interactive",
                "--format",
                "json",
                "--config",
                self.config.to_str().unwrap(),
                "--connections",
                self.connections.to_str().unwrap(),
            ];
            all.extend_from_slice(args);
            all
        })
    }

    /// Same run, text format: exact stdout shaping matters for the
    /// export/import/show byte-comparison tests.
    fn run_text(&self, args: &[&str]) -> Output {
        self.run_args(&{
            let mut all = vec![
                "--non-interactive",
                "--format",
                "text",
                "--config",
                self.config.to_str().unwrap(),
                "--connections",
                self.connections.to_str().unwrap(),
            ];
            all.extend_from_slice(args);
            all
        })
    }

    fn run_args(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_saya"))
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
fn saved_id(h: &Harness, label: &str) -> String {
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
        "save failed: {}{}",
        h.stderr(&save),
        h.stdout(&save)
    );
    let mut ids = h.document_ids();
    assert_eq!(ids.len(), 1, "exactly one document for {label}");
    ids.swap_remove(0)
}

/// `show` in text format prints the definition JSON, then the binding line.
/// Asserts both halves exactly against the exported file's bytes.
fn assert_show_matches_export(h: &Harness, id: &str, exported: &Path, binding_line: &str) {
    let show = h.run_text(&["investigation", "show", id]);
    assert_eq!(
        show.status.code(),
        Some(0),
        "show failed: {}{}",
        h.stderr(&show),
        h.stdout(&show)
    );
    let shown = h.stdout(&show);
    let exported_text = fs::read_to_string(exported).unwrap();
    let exported_text = exported_text.strip_suffix('\n').unwrap_or(&exported_text);
    let expected = format!("{exported_text}\n{binding_line}\n");
    assert_eq!(shown, expected, "show must print the exported definition");
}

include!("investigation_share/round_trip.rs");

include!("investigation_share/import_validation.rs");

include!("investigation_share/review_and_export.rs");
include!("investigation_share/fixture_identity.rs");
