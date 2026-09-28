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
    let root = std::env::temp_dir().join(format!("saya-investigation-share-db-{label}"));
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

#[test]
fn export_then_import_into_a_clean_root() {
    let a = harness_with_database("share-a");
    let id = saved_id(&a, "share-a");

    let exported = a.root.join("shared.json");
    let out = a.run(&["investigation", "export", &id, exported.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "export failed: {}{}",
        a.stderr(&out),
        a.stdout(&out)
    );

    // The exported file is the portable definition only.
    let exported_text = fs::read_to_string(&exported).unwrap();
    assert!(exported_text.contains("SELECT id, label FROM events ORDER BY id"));
    for forbidden in ["profile_identity", "local binding", "password"] {
        assert!(
            !exported_text.contains(forbidden),
            "export must not carry {forbidden:?}: {exported_text}"
        );
    }

    // A clean second root: import, then show reports no binding and the
    // identical definition.
    let b = harness_with_database("share-b");
    let imported = b.run(&["investigation", "import", exported.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(0),
        "import failed: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    let preview = b.stdout(&imported);
    assert!(
        preview.contains(
            "Imported without a local connection. Run with --connection <profile> to map it; nothing was executed."
        ),
        "preview: {preview}"
    );
    assert!(preview.contains(&id), "preview names the id: {preview}");
    assert!(
        preview.contains("Order events"),
        "preview names the investigation: {preview}"
    );
    assert!(
        preview.contains("SELECT id, label FROM events ORDER BY id"),
        "preview shows the exact SQL: {preview}"
    );
    assert!(
        preview.contains("events"),
        "preview lists objects: {preview}"
    );

    // The stored document is the exported bytes, exactly.
    let stored = fs::read(b.document_path(&id)).unwrap();
    assert_eq!(
        stored,
        fs::read(&exported).unwrap(),
        "import must store the exported bytes exactly"
    );

    assert_show_matches_export(&b, &id, &exported, "local binding: none");
    // And in the exporting root the binding is still recorded there.
    assert!(
        a.binding_path(&id).exists(),
        "export left A's binding alone"
    );
}

#[test]
fn import_never_executes_or_grants() {
    // The profile names a database path that does not exist: any connection
    // or query attempt would create it, so its absence proves import never
    // connected.
    let b = harness(
        "no-exec",
        &format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            std::env::temp_dir()
                .join(format!("saya-share-noexec-db-{}", std::process::id()))
                .display()
        ),
    );
    let file = definition_file(&b.root, "order.json", DEFINITION_JSON);
    let imported = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(0),
        "import failed: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    let db_path = std::env::temp_dir().join(format!("saya-share-noexec-db-{}", std::process::id()));
    assert!(
        !db_path.exists(),
        "import must never connect to the profile's database"
    );
    assert!(
        !b.investigations.join("local").exists(),
        "import must never write a binding"
    );

    let id = "order-events-01234567";
    let show = b.run(&["investigation", "show", id]);
    assert_eq!(show.status.code(), Some(0), "{}", b.stderr(&show));
    assert!(
        b.stdout(&show).contains("local binding: none"),
        "imported without a binding: {}",
        b.stdout(&show)
    );
    let _ = fs::remove_dir_all(&b.root);
}

#[test]
fn import_rejects_write_sql() {
    let b = harness_with_database("write-sql");
    let write_doc = DEFINITION_JSON.replace(
        "SELECT id, label FROM events ORDER BY id",
        "DELETE FROM events",
    );
    let file = definition_file(&b.root, "wipe.json", &write_doc);
    let imported = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(4),
        "the safety gate refuses write SQL: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    assert!(
        b.stderr(&imported).contains("read-only safety policy"),
        "err: {}",
        b.stderr(&imported)
    );
    assert!(
        b.document_ids().is_empty(),
        "a refused import stores nothing"
    );
    let _ = fs::remove_dir_all(&b.root);
}

#[test]
fn import_rejects_oversize_and_unknown_version() {
    let b = harness_with_database("oversize");

    let padded = format!("{DEFINITION_JSON}{}", " ".repeat(131_072));
    let file = definition_file(&b.root, "big.json", &padded);
    let imported = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(2),
        "oversize is a usage error: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    assert!(
        b.stderr(&imported).contains("131072"),
        "the refusal names the byte cap: {}",
        b.stderr(&imported)
    );
    assert!(b.document_ids().is_empty());

    let newer = DEFINITION_JSON.replace("\"version\": 1", "\"version\": 2");
    let file = definition_file(&b.root, "newer.json", &newer);
    let imported = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(2),
        "an unknown major version is refused: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    assert!(
        b.stderr(&imported)
            .contains("unsupported investigation version 2"),
        "err: {}",
        b.stderr(&imported)
    );
    assert!(b.document_ids().is_empty());
    let _ = fs::remove_dir_all(&b.root);
}

#[test]
fn import_is_idempotent_for_identical_content() {
    let b = harness_with_database("idempotent");
    let file = definition_file(&b.root, "order.json", DEFINITION_JSON);

    let first = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(first.status.code(), Some(0), "{}", b.stderr(&first));
    let id = "order-events-01234567";
    let stored = fs::read(b.document_path(id)).unwrap();

    let second = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(
        second.status.code(),
        Some(0),
        "identical re-import is a no-op: {}{}",
        b.stderr(&second),
        b.stdout(&second)
    );
    assert!(
        b.stdout(&second).contains("Already present and identical"),
        "the no-op says so: {}",
        b.stdout(&second)
    );
    assert_eq!(b.document_ids(), vec![id.to_string()]);
    assert_eq!(
        fs::read(b.document_path(id)).unwrap(),
        stored,
        "the stored document is unchanged"
    );
    let _ = fs::remove_dir_all(&b.root);
}

#[test]
fn import_conflict_for_same_id_different_content() {
    let b = harness_with_database("conflict");
    let file = definition_file(&b.root, "order.json", DEFINITION_JSON);
    let first = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(first.status.code(), Some(0), "{}", b.stderr(&first));
    let id = "order-events-01234567";
    let stored = fs::read(b.document_path(id)).unwrap();

    let renamed =
        DEFINITION_JSON.replace("\"name\": \"Order events\"", "\"name\": \"Renamed events\"");
    assert_ne!(renamed, DEFINITION_JSON, "mutation sanity");
    let other = definition_file(&b.root, "renamed.json", &renamed);
    let second = b.run(&["investigation", "import", other.to_str().unwrap()]);
    assert_eq!(
        second.status.code(),
        Some(2),
        "same id, different content is a conflict: {}{}",
        b.stderr(&second),
        b.stdout(&second)
    );
    assert!(
        b.stderr(&second)
            .contains("already exists with different content"),
        "err: {}",
        b.stderr(&second)
    );
    assert_eq!(
        fs::read(b.document_path(id)).unwrap(),
        stored,
        "the conflict changes nothing"
    );
    let _ = fs::remove_dir_all(&b.root);
}

/// Exports `id` from `a` and rewrites its `objects` field to `objects` in a
/// second file, as an editor (or a lying exporter) would hand a document in.
fn exported_with_objects(a: &Harness, id: &str, objects: serde_json::Value) -> PathBuf {
    let exported = a.root.join("shared.json");
    let out = a.run(&["investigation", "export", id, exported.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "export failed: {}{}",
        a.stderr(&out),
        a.stdout(&out)
    );
    let mut document: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&exported).unwrap()).unwrap();
    document["objects"] = objects;
    let tampered = a.root.join("tampered.json");
    fs::write(&tampered, serde_json::to_string_pretty(&document).unwrap()).unwrap();
    tampered
}

#[test]
fn imported_empty_objects_cannot_skip_schema_review() {
    // The audit repro (A2): a document whose objects list is emptied must not
    // import, because run takes its review dependencies from the SQL and an
    // empty stored list could otherwise make a changed table replay cleanly.
    let a = harness_with_database("empty-objects-a");
    let id = saved_id(&a, "empty-objects-a");
    let tampered = exported_with_objects(&a, &id, serde_json::json!([]));

    let b = harness_with_database("empty-objects-b");
    let imported = b.run(&["investigation", "import", tampered.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(2),
        "the emptied objects list is refused: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    assert!(
        b.stderr(&imported)
            .contains("objects do not match the SQL; re-export the investigation"),
        "err: {}",
        b.stderr(&imported)
    );
    assert!(
        b.document_ids().is_empty(),
        "a refused import stores nothing"
    );
    let _ = fs::remove_dir_all(&a.root);
    let _ = fs::remove_dir_all(&b.root);
}

#[test]
fn imported_wrong_objects_are_refused() {
    let a = harness_with_database("wrong-objects-a");
    let id = saved_id(&a, "wrong-objects-a");
    let tampered = exported_with_objects(&a, &id, serde_json::json!(["somewhere.else"]));

    let b = harness_with_database("wrong-objects-b");
    let imported = b.run(&["investigation", "import", tampered.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(2),
        "objects that disagree with the SQL are refused: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    assert!(
        b.stderr(&imported)
            .contains("objects do not match the SQL; re-export the investigation"),
        "err: {}",
        b.stderr(&imported)
    );
    assert!(b.document_ids().is_empty());
    let _ = fs::remove_dir_all(&a.root);
    let _ = fs::remove_dir_all(&b.root);
}

#[test]
fn export_refuses_existing_destination() {
    let a = harness_with_database("export-exists");
    let id = saved_id(&a, "export-exists");
    let dest = a.root.join("shared.json");
    let first = a.run(&["investigation", "export", &id, dest.to_str().unwrap()]);
    assert_eq!(first.status.code(), Some(0), "{}", a.stderr(&first));
    let exported = fs::read(&dest).unwrap();

    let second = a.run(&["investigation", "export", &id, dest.to_str().unwrap()]);
    assert_eq!(
        second.status.code(),
        Some(2),
        "an existing destination is refused: {}{}",
        a.stderr(&second),
        a.stdout(&second)
    );
    assert!(
        a.stderr(&second).contains("exists; pass --overwrite"),
        "err: {}",
        a.stderr(&second)
    );
    assert_eq!(
        fs::read(&dest).unwrap(),
        exported,
        "the refusal preserves the destination"
    );

    let over = a.run(&[
        "investigation",
        "export",
        &id,
        dest.to_str().unwrap(),
        "--overwrite",
    ]);
    assert_eq!(
        over.status.code(),
        Some(0),
        "--overwrite replaces: {}{}",
        a.stderr(&over),
        a.stdout(&over)
    );
    let _ = fs::remove_dir_all(&a.root);
}

#[test]
fn failed_export_preserves_destination() {
    let a = harness_with_database("export-failed");
    let id = saved_id(&a, "export-failed");

    // A directory destination is refused; its contents stay untouched and
    // no staging temp file remains anywhere in it.
    let outdir = a.root.join("outdir");
    fs::create_dir_all(&outdir).unwrap();
    fs::write(outdir.join("keep.txt"), b"keep").unwrap();
    let refused = a.run(&["investigation", "export", &id, outdir.to_str().unwrap()]);
    assert_eq!(
        refused.status.code(),
        Some(2),
        "a directory destination is refused: {}{}",
        a.stderr(&refused),
        a.stdout(&refused)
    );
    assert!(
        a.stderr(&refused).contains("is a directory"),
        "err: {}",
        a.stderr(&refused)
    );
    assert!(outdir.join("keep.txt").exists(), "directory unchanged");
    assert!(list_tmp_files(&outdir).is_empty(), "no temp file remains");

    // A symlink destination is refused even though its target is writable.
    let link = a.root.join("linked.json");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outdir.join("keep.txt"), &link).unwrap();
    let refused = a.run(&["investigation", "export", &id, link.to_str().unwrap()]);
    assert_eq!(
        refused.status.code(),
        Some(2),
        "a symlink destination is refused: {}{}",
        a.stderr(&refused),
        a.stdout(&refused)
    );
    assert!(
        a.stderr(&refused).contains("symlink"),
        "err: {}",
        a.stderr(&refused)
    );

    // An I/O failure (parent directory missing) leaves nothing behind and
    // refuses with exit 2.
    let missing = a.root.join("nosuchdir").join("out.json");
    let failed = a.run(&["investigation", "export", &id, missing.to_str().unwrap()]);
    assert_eq!(
        failed.status.code(),
        Some(2),
        "an unwritable destination is refused: {}{}",
        a.stderr(&failed),
        a.stdout(&failed)
    );
    assert!(!a.root.join("nosuchdir").exists(), "nothing was created");
    assert!(list_tmp_files(&a.root).is_empty(), "no temp file remains");

    // A missing id is refused before any path is touched.
    let refused = a.run(&[
        "investigation",
        "export",
        "no-such-id-00000000",
        a.root.join("x.json").to_str().unwrap(),
    ]);
    assert_eq!(refused.status.code(), Some(2), "{}", a.stderr(&refused));
    assert!(
        a.stderr(&refused)
            .contains("no investigation no-such-id-00000000"),
        "err: {}",
        a.stderr(&refused)
    );
    let _ = fs::remove_dir_all(&a.root);
}

fn list_tmp_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "tmp") {
                found.push(path);
            }
        }
    }
    found
}
