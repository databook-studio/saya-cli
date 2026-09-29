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

/// A missing required parameter refuses with the required names and types
/// before the run touches a profile: an unknown `--connection` would be the
/// first failure of any later step, so the parameter refusal winning over it
/// proves the check ran before connection work.
#[tokio::test]
async fn missing_parameter_fails_before_connect() {
    let h = harness_with_database("missing-required");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "By label",
        "SELECT id FROM events WHERE label = :label",
        &["label:string:required"],
    );

    let output = h.run(&[
        "investigation",
        "run",
        &id,
        "--connection",
        "no-such-profile",
    ]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "the missing required parameter refuses: {}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stderr = h.stderr(&output);
    assert!(
        stderr.contains("missing required parameter") && stderr.contains("label (string)"),
        "the refusal lists the required names with types: {stderr}"
    );
    assert!(
        stderr.contains("--param"),
        "the refusal names the remedy: {stderr}"
    );
    assert!(
        !stderr.contains("unknown profile"),
        "the refusal is not the profile error — it fired first: {stderr}"
    );
    assert!(
        !h.stdout(&output).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&output)
    );
    let _ = fs::remove_dir_all(h.root);
}

/// An unknown `--param` name refuses before connecting, naming the declared
/// parameters — never a value.
#[tokio::test]
async fn unknown_parameter_name_fails_before_connect() {
    let h = harness_with_database("unknown-param");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "By label",
        "SELECT id FROM events WHERE label = :label",
        &["label:string"],
    );

    let output = h.run(&[
        "investigation",
        "run",
        &id,
        "--connection",
        "no-such-profile",
        "--param",
        "region=east",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", h.stderr(&output));
    let stderr = h.stderr(&output);
    assert!(
        stderr.contains("region") && stderr.contains("declared"),
        "the refusal names the unknown parameter and the declared ones: {stderr}"
    );
    assert!(
        !stderr.contains("unknown profile"),
        "the refusal is not the profile error — it fired first: {stderr}"
    );
    let _ = fs::remove_dir_all(h.root);
}

/// A value that does not parse as its declared type refuses before
/// connecting; the error text never echoes the value.
#[tokio::test]
async fn value_parse_failure_refuses_before_connect() {
    let h = harness_with_database("bad-value");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "Since day",
        "SELECT id FROM events WHERE id > :since",
        &["since:date"],
    );

    let output = h.run(&[
        "investigation",
        "run",
        &id,
        "--connection",
        "no-such-profile",
        "--param",
        "since=not-a-date",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", h.stderr(&output));
    let stderr = h.stderr(&output);
    assert!(
        stderr.contains("since") && stderr.contains("date"),
        "the refusal names the parameter and its declared type: {stderr}"
    );
    assert!(
        !stderr.contains("not-a-date"),
        "a value must never leak into the error text: {stderr}"
    );
    assert!(
        !stderr.contains("unknown profile"),
        "the refusal is not the profile error — it fired first: {stderr}"
    );
    let _ = fs::remove_dir_all(h.root);
}

// ---------------------------------------------------------------------------
// invariant 2: capability honesty
// ---------------------------------------------------------------------------

/// A parameterized replay binds natively on SQLite and returns the filtered
/// rows; the parameters ride `QueryRequest::with_params`, not the SQL text.
#[tokio::test]
async fn parameterized_replay_binds_natively_on_sqlite() {
    let h = harness_with_database("sqlite-pass");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "By label",
        "SELECT id FROM events WHERE label = :label ORDER BY id",
        &["label:string"],
    );

    let output = h.run(&["investigation", "run", &id, "--param", "label=second"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stdout = h.stdout(&output);
    assert!(
        stdout.contains("\"event\":\"query_result\""),
        "the replay executed: {stdout}"
    );
    assert!(
        stdout.contains("\"row_count\":1") && stdout.contains("\"rows\":[[2]]"),
        "exactly the bound row came back: {stdout}"
    );
    let _ = fs::remove_dir_all(h.root);
}

/// Capability honesty: a DuckDB pass (native binds, no server needed), and
/// a ClickHouse-dialect definition refused honestly before any connection
/// attempt (the profile points nowhere). The ClickHouse definition is
/// constructed directly on disk — ClickHouse cannot be reached from here.
#[tokio::test]
async fn unsupported_binding_capability_is_honest() {
    let duck_dir = std::env::temp_dir().join(format!(
        "saya-investigation-params-duck-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&duck_dir);
    fs::create_dir_all(&duck_dir).unwrap();
    let duck_file = duck_dir.join("data.duckdb");
    {
        let conn = duckdb::Connection::open(&duck_file).unwrap();
        conn.execute_batch(
            "CREATE TABLE events (id INTEGER, label VARCHAR);
             INSERT INTO events VALUES (1, 'first'), (2, 'second');",
        )
        .unwrap();
    }
    let h = harness(
        "capability",
        format!(
            "[profiles.duck]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n\n\
             [profiles.ch]\ntype = 'clickhouse'\nhost = '127.0.0.1'\nport = 1\ndatabase = 'db'\nuser = 'u'\n",
            duck_file.display()
        ),
    );
    // The default profile exists, so composition resolves.
    fs::write(
        &h.config,
        "default_profile = 'duck'\n\n[run]\nmax_rows = 50\n",
    )
    .unwrap();

    // The DuckDB pass: the parameterized replay binds natively.
    let id = h.saved_id_for(
        "By label",
        "SELECT id FROM events WHERE label = :label ORDER BY id",
        &["label:string"],
        Some("duck"),
    );
    let pass = h.run(&[
        "investigation",
        "run",
        &id,
        "--connection",
        "duck",
        "--param",
        "label=first",
    ]);
    assert_eq!(
        pass.status.code(),
        Some(0),
        "DuckDB binds natively: {}{}",
        h.stdout(&pass),
        h.stderr(&pass)
    );
    assert!(
        h.stdout(&pass).contains("\"rows\":[[1]]"),
        "exactly the bound row came back: {}",
        h.stdout(&pass)
    );

    // The ClickHouse-dialect definition, written directly as the task allows:
    // its placeholders are declared, but no ClickHouse server is reachable,
    // so any connect attempt would fail with a connection error — the honest
    // capability refusal must win first.
    let id = "by-region-01234567";
    fs::write(
        h.document_path(id),
        r#"{
  "format": "saya.investigation",
  "version": 1,
  "id": "by-region-01234567",
  "revision": 1,
  "name": "By region",
  "sql": "SELECT 1 WHERE x = :region",
  "parameters": [{"name": "region", "type": "string", "required": false}],
  "dialect": "clickhouse",
  "connection": "ch",
  "objects": [],
  "schema_fingerprint": null,
  "created_unix_ms": 1700000000000,
  "updated_unix_ms": 1700000000000
}"#,
    )
    .unwrap();

    let output = h.run(&[
        "investigation",
        "run",
        id,
        "--connection",
        "ch",
        "--param",
        "region=east",
    ]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "the unsupported engine refuses: {}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stderr = h.stderr(&output);
    assert!(
        stderr.contains("parameters are not supported for ClickHouse"),
        "the refusal names the engine honestly: {stderr}"
    );
    assert!(
        !h.stdout(&output).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&output)
    );
    let _ = fs::remove_dir_all(h.root);
    let _ = fs::remove_dir_all(duck_dir);
}

// ---------------------------------------------------------------------------
// invariant 1: save and edit declare; the placeholders must match exactly
// ---------------------------------------------------------------------------

/// Saving SQL whose `:name` placeholders are not declared refuses, naming
/// them; declaring a parameter the SQL does not use refuses too.
#[tokio::test]
async fn save_refuses_undeclared_placeholders_in_both_directions() {
    let h = harness_with_database("undeclared");
    seed_events(&h.database).await;

    let undeclared = h.run(&[
        "investigation",
        "save",
        "--name",
        "Missing spec",
        "--sql",
        "SELECT id FROM events WHERE label = :label",
    ]);
    assert_eq!(
        undeclared.status.code(),
        Some(2),
        "{}{}",
        h.stdout(&undeclared),
        h.stderr(&undeclared)
    );
    let stderr = h.stderr(&undeclared);
    assert!(
        stderr.contains("label") && stderr.contains("--param-spec"),
        "the refusal names the placeholder and the declaring flag: {stderr}"
    );

    let extra = h.run(&[
        "investigation",
        "save",
        "--name",
        "Extra spec",
        "--sql",
        "SELECT id FROM events WHERE label = :label",
        "--param-spec",
        "region:string",
    ]);
    assert_eq!(
        extra.status.code(),
        Some(2),
        "{}{}",
        h.stdout(&extra),
        h.stderr(&extra)
    );
    let stderr = h.stderr(&extra);
    assert!(
        stderr.contains("label") && stderr.contains("region"),
        "the refusal names both the missing and the extra name: {stderr}"
    );

    // A malformed spec is a usage refusal, not a save.
    let bad_type = h.run(&[
        "investigation",
        "save",
        "--name",
        "Bad type",
        "--sql",
        "SELECT :label",
        "--param-spec",
        "label:str",
    ]);
    assert_eq!(bad_type.status.code(), Some(2), "{}", h.stderr(&bad_type));
    let stderr = h.stderr(&bad_type);
    assert!(
        stderr.contains("string|integer|boolean|decimal|date|timestamp"),
        "the refusal names the valid types: {stderr}"
    );
    let _ = fs::remove_dir_all(h.root);
}

/// A save with matching declarations stores them in the definition; the run
/// binds them.
#[tokio::test]
async fn save_stores_the_declared_parameters() {
    let h = harness_with_database("declared");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "Since day",
        "SELECT id FROM events WHERE id > :since AND label = :label",
        &["since:date", "label:string:required"],
    );

    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    assert!(
        document.contains("\"parameters\"")
            && document.contains("\"type\": \"date\"")
            && document.contains("\"required\": true"),
        "the declared parameters persist in the definition: {document}"
    );
    assert!(
        !document.contains("params_sha256"),
        "a definition never carries a value digest: {document}"
    );
    let _ = fs::remove_dir_all(h.root);
}

// ---------------------------------------------------------------------------
// invariant 2: edit replaces the spec list, bumps the revision, stales
// ---------------------------------------------------------------------------

#[tokio::test]
async fn edit_replaces_the_spec_bumps_the_revision_and_stales() {
    let h = harness_with_database("edit-spec");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "By label",
        "SELECT id FROM events WHERE label = :label",
        &["label:string"],
    );

    // Editing only the spec list is a valid edit: the placeholders must keep
    // matching the SQL.
    let edited = h.run(&[
        "investigation",
        "edit",
        &id,
        "--param-spec",
        "label:string:required",
    ]);
    assert_eq!(
        edited.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&edited),
        h.stderr(&edited)
    );
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    assert!(
        document.contains("\"revision\": 2") && document.contains("\"required\": true"),
        "the edit published revision 2 with the replaced spec: {document}"
    );

    // The spec change is a new revision, so the review is stale.
    let stale = h.run(&["investigation", "run", &id, "--param", "label=first"]);
    assert_eq!(stale.status.code(), Some(2), "{}", h.stderr(&stale));
    let stderr = h.stderr(&stale);
    assert!(
        stderr.contains("revision changed"),
        "the stale refusal names what changed: {stderr}"
    );

    // An edit whose specs no longer cover the SQL refuses, naming the
    // placeholder.
    let uncovered = h.run(&["investigation", "edit", &id, "--param-spec", "day:date"]);
    assert_eq!(
        uncovered.status.code(),
        Some(2),
        "{}{}",
        h.stdout(&uncovered),
        h.stderr(&uncovered)
    );
    let stderr = h.stderr(&uncovered);
    assert!(
        stderr.contains("label"),
        "the refusal names the now-undeclared placeholder: {stderr}"
    );

    let revalidated = h.run(&[
        "investigation",
        "run",
        &id,
        "--param",
        "label=first",
        "--revalidate",
    ]);
    assert_eq!(
        revalidated.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&revalidated),
        h.stderr(&revalidated)
    );
    let _ = fs::remove_dir_all(h.root);
}

// ---------------------------------------------------------------------------
// invariant 3: values never persist
// ---------------------------------------------------------------------------

#[tokio::test]
async fn parameter_values_never_persist() {
    const SENTINEL: &str = "HVNS3NT1NEL42";
    let h = harness_with_database("no-persist");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "By label",
        "SELECT id FROM events WHERE label = :label",
        &["label:string"],
    );
    let report = h.root.join("report.md");

    let output = h.run(&[
        "investigation",
        "run",
        &id,
        "--param",
        &format!("label={SENTINEL}"),
        "--report",
        report.to_str().unwrap(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stdout = h.stdout(&output);
    let stderr = h.stderr(&output);
    assert!(
        !stdout.contains(SENTINEL) && !stderr.contains(SENTINEL),
        "the value never reaches stdout or stderr: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("params: label"),
        "the evidence line records the parameter NAME: {stdout}"
    );

    // Every file beneath the harness root — the definition, the binding, the
    // state/audit database, the config and connections files, the session
    // state, and the report — is scanned for the value.
    let mut offenders = Vec::new();
    scan_for(&h.root, SENTINEL, &mut offenders);
    assert!(
        offenders.is_empty(),
        "the bound value must never persist on disk: {offenders:?}"
    );
    assert!(report.exists(), "the report was written");

    // The definition still holds the declaration, not the value.
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    assert!(!document.contains(SENTINEL), "{document}");
    let binding = fs::read_to_string(h.binding_path(&id)).unwrap();
    assert!(!binding.contains(SENTINEL), "{binding}");
    let _ = fs::remove_dir_all(h.root);
}

fn scan_for(dir: &Path, needle: &str, offenders: &mut Vec<PathBuf>) {
    let needle = needle.as_bytes();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            scan_for(&path, &String::from_utf8_lossy(needle), offenders);
        } else if let Ok(bytes) = fs::read(&path)
            && bytes.windows(needle.len()).any(|window| window == needle)
        {
            offenders.push(path);
        }
    }
}

// ---------------------------------------------------------------------------
// invariant 2: a value stays data, a typed null binds
// ---------------------------------------------------------------------------

/// A value carrying a quote and a SQL comment binds as a literal: zero rows,
/// never every row.
#[tokio::test]
async fn a_quote_and_comment_laden_value_stays_data() {
    let h = harness_with_database("injection");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "By label",
        "SELECT id FROM events WHERE label = :label",
        &["label:string"],
    );

    let output = h.run(&["investigation", "run", &id, "--param", "label=' OR 1=1 --"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stdout = h.stdout(&output);
    assert!(
        stdout.contains("\"row_count\":0") && stdout.contains("\"rows\":[]"),
        "the value matched nothing — it was data, not SQL: {stdout}"
    );
    assert!(
        !stdout.contains("first") && !stdout.contains("second"),
        "no seeded row leaked: {stdout}"
    );
    let _ = fs::remove_dir_all(h.root);
}

/// An explicitly typed null (`--param name=null`) and an omitted optional
/// parameter both bind as nulls on SQLite, and the run succeeds.
#[tokio::test]
async fn typed_null_binds_on_sqlite() {
    let h = harness_with_database("typed-null");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "Null probes",
        "SELECT :v IS NULL AS v_null, :w IS NULL AS w_null",
        &["v:date", "w:string:required"],
    );

    let explicit = h.run(&[
        "investigation",
        "run",
        &id,
        "--param",
        "v=null",
        "--param",
        "w=null",
    ]);
    assert_eq!(
        explicit.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&explicit),
        h.stderr(&explicit)
    );
    let stdout = h.stdout(&explicit);
    assert!(
        stdout.contains("\"rows\":[[1,1]]"),
        "both typed nulls bound: {stdout}"
    );

    // An optional parameter left out binds as a typed null too; the supplied
    // one binds as its value.
    let omitted = h.run(&["investigation", "run", &id, "--param", "w=first"]);
    assert_eq!(
        omitted.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&omitted),
        h.stderr(&omitted)
    );
    assert!(
        h.stdout(&omitted).contains("\"rows\":[[1,0]]"),
        "the omitted optional parameter bound as a typed null: {}",
        h.stdout(&omitted)
    );
    let _ = fs::remove_dir_all(h.root);
}

// ---------------------------------------------------------------------------
// invariant 1: import validates the placeholder/declaration contract too
// ---------------------------------------------------------------------------

/// An imported document whose placeholders and declarations disagree is
/// refused, naming them — a hand-written file gets the same gate save does.
#[tokio::test]
async fn import_refuses_a_placeholder_declaration_mismatch() {
    let h = harness_with_database("import-contract");
    seed_events(&h.database).await;

    let mismatched = r#"{
  "format": "saya.investigation",
  "version": 1,
  "id": "mismatched-01234567",
  "revision": 1,
  "name": "Mismatched",
  "sql": "SELECT id FROM events WHERE label = :label",
  "parameters": [{"name": "region", "type": "string", "required": false}],
  "dialect": "sqlite",
  "connection": "local",
  "objects": ["events"],
  "schema_fingerprint": null,
  "created_unix_ms": 1700000000000,
  "updated_unix_ms": 1700000000000
}"#;
    let path = h.root.join("mismatched.json");
    fs::write(&path, mismatched).unwrap();

    let output = h.run(&["investigation", "import", path.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stderr = h.stderr(&output);
    assert!(
        stderr.contains("label") && stderr.contains("region"),
        "the refusal names the missing and the extra name: {stderr}"
    );

    // A matching document imports.
    let matching = r#"{
  "format": "saya.investigation",
  "version": 1,
  "id": "matched-01234567",
  "revision": 1,
  "name": "Matched",
  "sql": "SELECT id FROM events WHERE label = :label",
  "parameters": [{"name": "label", "type": "string", "required": false}],
  "dialect": "sqlite",
  "connection": "local",
  "objects": ["events"],
  "schema_fingerprint": null,
  "created_unix_ms": 1700000000000,
  "updated_unix_ms": 1700000000000
}"#;
    let path = h.root.join("matched.json");
    fs::write(&path, matching).unwrap();
    let output = h.run(&["investigation", "import", path.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let _ = fs::remove_dir_all(h.root);
}
