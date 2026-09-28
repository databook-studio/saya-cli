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

#[test]
fn investigation_help_lists_subcommands() {
    let h = harness_with_database("help");
    let help = h.run(&["investigation", "--help"]);
    assert_eq!(help.status.code(), Some(0), "{}", h.stderr(&help));
    let stdout = h.stdout(&help);
    for expected in ["save", "list", "show", "delete"] {
        assert!(
            stdout.contains(expected),
            "help must list {expected}: {stdout}"
        );
    }
    let save_help = h.run(&["investigation", "save", "--help"]);
    assert_eq!(save_help.status.code(), Some(0), "{}", h.stderr(&save_help));
    let save_stdout = h.stdout(&save_help);
    for expected in ["--sql", "--file", "--connection", "--name", "--description"] {
        assert!(
            save_stdout.contains(expected),
            "save help must list {expected}: {save_stdout}"
        );
    }
}

#[test]
fn save_then_list_and_show_roundtrip() {
    let h = harness_with_database("roundtrip");

    let empty = h.run(&["investigation", "list"]);
    assert_eq!(empty.status.code(), Some(0));
    assert!(h.stdout(&empty).contains("No saved investigations."));

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
        h.stdout(&save),
        h.stderr(&save)
    );
    let stdout = h.stdout(&save);
    assert!(
        stdout.contains(
            "Saved exactly as shown. Review the SQL before sharing: literals are stored verbatim."
        ),
        "{stdout}"
    );
    let ids = h.document_ids();
    assert_eq!(ids.len(), 1, "exactly one document written: {ids:?}");
    let id = ids[0].clone();
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    assert!(document.contains("SELECT id, label FROM events ORDER BY id"));
    assert!(
        h.binding_path(&id).exists(),
        "binding written beside the document"
    );

    let list = h.run(&["investigation", "list"]);
    assert_eq!(list.status.code(), Some(0));
    assert!(
        h.stdout(&list)
            .contains(&format!("{id}  1  sqlite  local  Order events")),
        "list line: {}",
        h.stdout(&list)
    );

    let show = h.run(&["investigation", "show", &id]);
    assert_eq!(show.status.code(), Some(0));
    let show_stdout = h.stdout(&show);
    assert!(show_stdout.contains("SELECT id, label FROM events ORDER BY id"));
    assert!(
        show_stdout.contains("local binding: local (reviewed revision 1)"),
        "{show_stdout}"
    );
    // The opaque profile identity never reaches the document or the output.
    assert!(!document.contains("profile_identity"), "{document}");
    assert!(!show_stdout.contains("profile_identity"), "{show_stdout}");
}

#[test]
fn save_refuses_write_sql() {
    let h = harness_with_database("write-sql");
    let save = h.run(&[
        "investigation",
        "save",
        "--name",
        "Wipe",
        "--sql",
        "DELETE FROM events",
    ]);
    assert_eq!(save.status.code(), Some(4), "{}", h.stderr(&save));
    assert!(
        h.stderr(&save).contains("read-only safety policy"),
        "the refusal names the safety gate: {}",
        h.stderr(&save)
    );
    assert!(h.document_ids().is_empty(), "nothing written");
}

#[test]
fn save_refuses_credential_shaped_sql() {
    let h = harness_with_database("credential");
    let save = h.run(&[
        "investigation",
        "save",
        "--name",
        "Leak",
        "--sql",
        "SELECT 'password=hunter2'",
    ]);
    assert_eq!(save.status.code(), Some(2), "{}", h.stderr(&save));
    assert!(
        h.stderr(&save).contains("credential-shaped"),
        "err: {}",
        h.stderr(&save)
    );
    assert!(h.document_ids().is_empty(), "nothing written");
}

#[test]
fn save_requires_a_connection() {
    // No profiles at all: nothing resolves, so save must refuse with the
    // flag to pass instead of guessing.
    let h = harness("no-connection", "");
    let save = h.run(&[
        "investigation",
        "save",
        "--name",
        "Lonely",
        "--sql",
        "SELECT 1",
    ]);
    assert_eq!(save.status.code(), Some(2), "{}", h.stderr(&save));
    assert!(
        h.stderr(&save)
            .contains("no connection: pass --connection <profile>"),
        "err: {}",
        h.stderr(&save)
    );
    assert!(h.document_ids().is_empty());
}

#[test]
fn definition_export_excludes_credentials_rows_and_grants() {
    let h = harness_with_database("sentinel");
    let save = h.run(&[
        "investigation",
        "save",
        "--name",
        "Clean events",
        "--description",
        "labelled events",
        "--sql",
        "SELECT id, label FROM events WHERE label IS NOT NULL",
    ]);
    assert_eq!(save.status.code(), Some(0), "{}", h.stderr(&save));
    let id = h.document_ids().into_iter().next().unwrap();
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    for forbidden in [
        "password",
        "token",
        "rows",
        "grant",
        "identity",
        "profile_identity",
    ] {
        assert!(
            !document.contains(forbidden),
            "the portable document must not contain {forbidden:?}: {document}"
        );
    }
    // The binding is per-machine state beside the document, never inside it.
    let binding = fs::read_to_string(h.binding_path(&id)).unwrap();
    assert!(
        binding.contains("\"profile_identity\""),
        "binding: {binding}"
    );
    assert!(!binding.contains("password"), "binding: {binding}");
}

#[test]
fn delete_removes_definition_and_binding() {
    let h = harness_with_database("delete");
    let id = saved_id(&h);

    let stale = h.run(&["investigation", "delete", &id, "--revision", "5"]);
    assert_eq!(stale.status.code(), Some(2), "{}", h.stderr(&stale));
    let stderr = h.stderr(&stale);
    assert!(
        stderr.contains("at revision 1"),
        "the refusal names the current revision: {stderr}"
    );
    assert!(
        h.document_path(&id).exists(),
        "a refused delete changes nothing"
    );

    let deleted = h.run(&["investigation", "delete", &id, "--revision", "1"]);
    assert_eq!(deleted.status.code(), Some(0), "{}", h.stderr(&deleted));
    assert!(!h.document_path(&id).exists(), "document removed");
    assert!(!h.binding_path(&id).exists(), "binding removed too");

    let show = h.run(&["investigation", "show", &id]);
    assert_eq!(show.status.code(), Some(2));
    assert!(
        h.stderr(&show).contains(&format!("no investigation {id}")),
        "err: {}",
        h.stderr(&show)
    );

    let list = h.run(&["investigation", "list"]);
    assert_eq!(list.status.code(), Some(0));
    assert!(h.stdout(&list).contains("No saved investigations."));
}

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
