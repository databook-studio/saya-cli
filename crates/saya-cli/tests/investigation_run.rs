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
    let root = std::env::temp_dir().join(format!("saya-investigation-db-{label}"));
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

/// Runs one DDL statement through sqlx, the schema change a stale review
/// must catch.
async fn alter_table(database: &Path, sql: &str) {
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(database);
    let pool = SqlitePool::connect_with(options).await.unwrap();
    sqlx::query(sql).execute(&pool).await.unwrap();
    pool.close().await;
}

/// A provider config that cannot be reached: any provider call fails fast.
fn provider_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("SAYA_PROVIDER", "openai_compatible"),
        ("SAYA_MODEL", "mock-model"),
        ("SAYA_PROVIDER_BASE_URL", "http://127.0.0.1:9/v1"),
        ("SAYA_API_KEY", "mock-secret"),
    ]
}

#[tokio::test]
async fn replay_does_not_call_provider() {
    let h = harness_with_database("provider");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);

    let output = h.run_env(&["investigation", "run", &id], &provider_env());
    assert_eq!(
        output.status.code(),
        Some(0),
        "replay must not need a provider: {}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stdout = h.stdout(&output);
    assert!(
        stdout.contains("\"event\":\"query_result\""),
        "the normal query result event renders: {stdout}"
    );
    assert!(
        stdout.contains("saved investigation"),
        "an evidence line with the saved source follows: {stdout}"
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn replay_cannot_fall_back_to_active_profile() {
    // The definition is written directly into the dir with no binding, as an
    // import would leave it; the single profile is still the active one.
    let source = harness_with_database("fallthrough-source");
    seed_events(&source.database).await;
    let id = saved_id(&source, None);
    let document = fs::read_to_string(source.document_path(&id)).unwrap();

    let h = harness_with_database("fallthrough");
    seed_events(&h.database).await;
    fs::write(h.document_path(&id), document).unwrap();

    let output = h.run(&["investigation", "run", &id]);
    assert_eq!(output.status.code(), Some(2), "{}", h.stderr(&output));
    assert!(
        h.stderr(&output)
            .contains("no local connection mapped: pass --connection <profile>"),
        "the refusal names the explicit flag: {}",
        h.stderr(&output)
    );
    assert!(
        !h.stdout(&output).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&output)
    );
    let _ = fs::remove_dir_all(source.root);
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn changed_schema_invalidates_review() {
    let h = harness_with_database("schema-drift");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);

    // First run: the save-time binding carries no fingerprint, so this run
    // completes the review with the live schema fingerprint.
    let first = h.run(&["investigation", "run", &id]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&first),
        h.stderr(&first)
    );
    let binding = fs::read_to_string(h.binding_path(&id)).unwrap();
    assert!(
        binding.contains("reviewed_schema_fingerprint"),
        "the first successful run binds the fingerprint: {binding}"
    );

    // A fresh, unchanged replay still passes.
    let second = h.run(&["investigation", "run", &id]);
    assert_eq!(
        second.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&second),
        h.stderr(&second)
    );

    // The referenced table changes underneath the review.
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&h.database);
    let pool = SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("ALTER TABLE events ADD COLUMN extra TEXT")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let stale = h.run(&["investigation", "run", &id]);
    assert_eq!(stale.status.code(), Some(2), "{}", h.stderr(&stale));
    let stderr = h.stderr(&stale);
    assert!(
        stderr.contains("schema changed"),
        "the refusal names what changed: {stderr}"
    );
    assert!(
        !h.stdout(&stale).contains("\"event\":\"query_result\""),
        "a stale review never executes: {}",
        h.stdout(&stale)
    );

    let revalidated = h.run(&["investigation", "run", &id, "--revalidate"]);
    assert_eq!(
        revalidated.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&revalidated),
        h.stderr(&revalidated)
    );

    // The rewritten binding makes plain replays pass again.
    let after = h.run(&["investigation", "run", &id]);
    assert_eq!(
        after.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&after),
        h.stderr(&after)
    );
    let _ = fs::remove_dir_all(h.root);
}

/// A hand-written v1 definition with an empty stored objects list, exactly
/// what a lying exporter could hand over (the audit repro's on-disk shape).
const EMPTY_OBJECTS_DEFINITION: &str = r#"{
  "format": "saya.investigation",
  "version": 1,
  "id": "order-events-01234567",
  "revision": 1,
  "name": "Order events",
  "sql": "SELECT id, label FROM events ORDER BY id",
  "dialect": "sqlite",
  "connection": "local",
  "objects": [],
  "schema_fingerprint": null,
  "created_unix_ms": 1700000000000,
  "updated_unix_ms": 1700000000000
}"#;

#[tokio::test]
async fn run_ignores_stored_objects_in_review() {
    let h = harness_with_database("ignored-objects");
    seed_events(&h.database).await;
    // The document lands on disk with `objects: []` and no binding. The
    // review must take its dependencies from the SQL, so the first replay
    // still binds the referenced table's fingerprint.
    let id = "order-events-01234567";
    fs::write(h.document_path(id), EMPTY_OBJECTS_DEFINITION).unwrap();

    let first = h.run(&["investigation", "run", id, "--connection", "local"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&first),
        h.stderr(&first)
    );

    alter_table(&h.database, "ALTER TABLE events ADD COLUMN extra TEXT").await;

    let stale = h.run(&["investigation", "run", id]);
    assert_eq!(
        stale.status.code(),
        Some(2),
        "the empty stored list must not skip the schema review: {}{}",
        h.stdout(&stale),
        h.stderr(&stale)
    );
    assert!(
        h.stderr(&stale).contains("schema changed"),
        "the refusal names what changed: {}",
        h.stderr(&stale)
    );
    assert!(
        !h.stdout(&stale).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&stale)
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn quoted_dotted_table_gets_a_real_fingerprint() {
    let h = harness_with_database("dotted-table");
    {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&h.database)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::query(r#"CREATE TABLE "orders.v1" (id INTEGER PRIMARY KEY, label TEXT NOT NULL)"#)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(r#"INSERT INTO "orders.v1" (id, label) VALUES (1, 'first')"#)
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let id = saved_sql_id(&h, r#"SELECT id, label FROM "orders.v1""#);

    // The document names the dotted table as one canonically quoted object,
    // never a schema/table split on the dot inside the name.
    let document: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(h.document_path(&id)).unwrap()).unwrap();
    assert_eq!(
        document["objects"],
        serde_json::json!([r#""orders.v1""#]),
        "objects: {document}"
    );

    // The replay resolves that one object and binds its real fingerprint.
    let first = h.run(&["investigation", "run", &id]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&first),
        h.stderr(&first)
    );
    let binding = fs::read_to_string(h.binding_path(&id)).unwrap();
    assert!(
        binding.contains("reviewed_schema_fingerprint"),
        "the dotted table contributed a real fingerprint: {binding}"
    );

    alter_table(
        &h.database,
        r#"ALTER TABLE "orders.v1" ADD COLUMN extra TEXT"#,
    )
    .await;

    let stale = h.run(&["investigation", "run", &id]);
    assert_eq!(
        stale.status.code(),
        Some(2),
        "a change to the dotted table invalidates the review: {}{}",
        h.stdout(&stale),
        h.stderr(&stale)
    );
    assert!(
        h.stderr(&stale).contains("schema changed"),
        "the refusal names what changed: {}",
        h.stderr(&stale)
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn missing_table_is_unverifiable_not_reviewed() {
    let h = harness_with_database("missing-table");
    // AUTOINCREMENT makes SQLite create sqlite_sequence: a live, queryable
    // table the schema discovery never lists, so the SQL names a table the
    // tree cannot resolve.
    {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&h.database)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::query(
            "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO events (label) VALUES ('first')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let id = saved_sql_id(&h, "SELECT * FROM sqlite_sequence");

    // A missing object is unverifiable: refused, and never recorded as a
    // reviewed fingerprint via a missing-name constant.
    let refused = h.run(&["investigation", "run", &id]);
    assert_eq!(
        refused.status.code(),
        Some(2),
        "{}{}",
        h.stdout(&refused),
        h.stderr(&refused)
    );
    let stderr = h.stderr(&refused);
    // The refusal is JSON-encoded, so the name's display quotes arrive
    // escaped.
    assert!(
        stderr.contains(r#"schema review unavailable: table \"sqlite_sequence\" not found"#),
        "{stderr}"
    );
    assert!(stderr.contains("--revalidate"), "{stderr}");
    assert!(
        !h.stdout(&refused).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&refused)
    );

    // With --revalidate the run proceeds — the query itself succeeds — and
    // the rewritten binding records no fingerprint: it claims no review.
    let revalidated = h.run(&["investigation", "run", &id, "--revalidate"]);
    assert_eq!(
        revalidated.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&revalidated),
        h.stderr(&revalidated)
    );
    let binding = fs::read_to_string(h.binding_path(&id)).unwrap();
    assert!(
        binding.contains("\"reviewed_schema_fingerprint\": null"),
        "the revalidated run must not claim a schema review: {binding}"
    );

    // The next run is unverifiable again.
    let again = h.run(&["investigation", "run", &id]);
    assert_eq!(
        again.status.code(),
        Some(2),
        "{}{}",
        h.stdout(&again),
        h.stderr(&again)
    );
    assert!(
        h.stderr(&again).contains("schema review unavailable"),
        "{}",
        h.stderr(&again)
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn select_without_tables_is_complete() {
    let h = harness_with_database("select-one");
    seed_events(&h.database).await;
    let id = saved_sql_id(&h, "SELECT 1");

    let output = h.run(&["investigation", "run", &id]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a query with no objects completes its review without --revalidate: {}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    assert!(
        h.stdout(&output).contains("\"event\":\"query_result\""),
        "{}",
        h.stdout(&output)
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn different_target_requires_revalidate() {
    let other_database = label_database("different-target-other");
    fs::write(&other_database, b"").unwrap();
    let h = harness(
        "different-target",
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n\n\
             [profiles.other]\ntype = 'sqlite'\npath = '{}'\n",
            label_database("different-target-local").display(),
            other_database.display()
        ),
    );
    let local_database = label_database("different-target-local");
    fs::write(&local_database, b"").unwrap();
    seed_events(&local_database).await;
    seed_events(&other_database).await;
    let id = saved_id(&h, Some("local"));

    let refused = h.run(&["investigation", "run", &id, "--connection", "other"]);
    assert_eq!(refused.status.code(), Some(2), "{}", h.stderr(&refused));
    let stderr = h.stderr(&refused);
    assert!(
        stderr.contains("target changed"),
        "a different profile is a stale review: {stderr}"
    );

    let revalidated = h.run(&[
        "investigation",
        "run",
        &id,
        "--connection",
        "other",
        "--revalidate",
    ]);
    assert_eq!(
        revalidated.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&revalidated),
        h.stderr(&revalidated)
    );

    let plain = h.run(&["investigation", "run", &id, "--connection", "other"]);
    assert_eq!(
        plain.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&plain),
        h.stderr(&plain)
    );
    let _ = fs::remove_dir_all(h.root);
    let _ = fs::remove_dir_all(local_database.parent().unwrap());
}

#[tokio::test]
async fn dialect_mismatch_refused() {
    let duck_database = label_database("dialect-duck");
    let h = harness(
        "dialect",
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n\n\
             [profiles.duck]\ntype = 'duckdb'\npath = '{}'\n",
            label_database("dialect-local").display(),
            duck_database.display()
        ),
    );
    let local_database = label_database("dialect-local");
    fs::write(&local_database, b"").unwrap();
    seed_events(&local_database).await;
    let id = saved_id(&h, Some("local"));

    let output = h.run(&["investigation", "run", &id, "--connection", "duck"]);
    assert_eq!(output.status.code(), Some(2), "{}", h.stderr(&output));
    let stderr = h.stderr(&output);
    assert!(
        stderr.contains("dialect mismatch"),
        "the refusal names the dialect: {stderr}"
    );
    assert!(
        !h.stdout(&output).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&output)
    );
    let _ = fs::remove_dir_all(h.root);
    let _ = fs::remove_dir_all(local_database.parent().unwrap());
}

#[tokio::test]
async fn investigation_replay_obeys_read_only_gate() {
    let h = harness_with_database("read-only");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);

    // The definition is rewritten directly on disk, bypassing save's gate.
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    fs::write(
        h.document_path(&id),
        document.replace(
            "SELECT id, label FROM events ORDER BY id",
            "DELETE FROM events",
        ),
    )
    .unwrap();

    let output = h.run(&["investigation", "run", &id]);
    assert_eq!(
        output.status.code(),
        Some(4),
        "the safety gate refuses the replay: {}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    assert!(
        h.stderr(&output).contains("read-only"),
        "the refusal names the gate: {}",
        h.stderr(&output)
    );
    assert_eq!(
        event_rows(&h.database).await,
        2,
        "the table is intact: nothing executed"
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn first_explicit_mapping_creates_the_binding() {
    let h = harness_with_database("first-mapping");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);
    fs::remove_file(h.binding_path(&id)).unwrap();

    // No binding: run refuses without an explicit target.
    let refused = h.run(&["investigation", "run", &id]);
    assert_eq!(refused.status.code(), Some(2), "{}", h.stderr(&refused));

    // The explicit mapping runs and creates the binding.
    let mapped = h.run(&["investigation", "run", &id, "--connection", "local"]);
    assert_eq!(
        mapped.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&mapped),
        h.stderr(&mapped)
    );
    let stdout = h.stdout(&mapped);
    assert!(
        stdout.contains("saved investigation: local"),
        "the evidence line names the profile: {stdout}"
    );
    assert!(
        h.binding_path(&id).exists(),
        "the mapping created the binding"
    );

    // After the binding exists, the plain run resolves its target from it.
    let plain = h.run(&["investigation", "run", &id]);
    assert_eq!(
        plain.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&plain),
        h.stderr(&plain)
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn run_refuses_missing_malformed_and_stale_revisions() {
    let h = harness_with_database("refusals");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);

    let malformed = h.run(&["investigation", "run", "NOT AN ID"]);
    assert_eq!(malformed.status.code(), Some(2), "{}", h.stderr(&malformed));
    assert!(
        h.stderr(&malformed).contains("no investigation"),
        "err: {}",
        h.stderr(&malformed)
    );

    let missing = h.run(&["investigation", "run", "ghost-00000000"]);
    assert_eq!(missing.status.code(), Some(2), "{}", h.stderr(&missing));
    assert!(
        h.stderr(&missing)
            .contains("no investigation ghost-00000000"),
        "err: {}",
        h.stderr(&missing)
    );

    // A definition revised on disk (revision 2, binding says 1) is stale.
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    fs::write(
        h.document_path(&id),
        document.replace("\"revision\": 1", "\"revision\": 2"),
    )
    .unwrap();
    let stale = h.run(&["investigation", "run", &id]);
    assert_eq!(stale.status.code(), Some(2), "{}", h.stderr(&stale));
    let stderr = h.stderr(&stale);
    assert!(
        stderr.contains("revision changed"),
        "the refusal names what changed: {stderr}"
    );
    assert!(
        !h.stdout(&stale).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&stale)
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn run_report_writes_sql_and_provenance_without_rows() {
    let h = harness_with_database("report-default");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);
    let report = h.root.join("report.md");

    let output = h.run(&[
        "investigation",
        "run",
        &id,
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
    let document = fs::read_to_string(&report).unwrap();
    assert!(document.contains("# saya report"), "{document}");
    assert!(document.contains("## Query"), "{document}");
    assert!(
        document.contains("SELECT id, label FROM events ORDER BY id"),
        "the exact saved SQL is in the report: {document}"
    );
    assert!(document.contains("## Provenance"), "{document}");
    assert!(document.contains("Connection label: local"), "{document}");
    assert!(document.contains("Submitted SQL sha256: "), "{document}");
    assert!(document.contains("Execution id: "), "{document}");
    assert!(document.contains("Scope: full result"), "{document}");
    assert!(
        document.contains("Rows omitted (pass --rows N to include up to 100)."),
        "{document}"
    );
    assert!(
        !document.contains("| id |"),
        "rows land only on --rows N: {document}"
    );
    assert!(
        !document.contains("| 1 | first |"),
        "no row data without --rows: {document}"
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn run_report_rows_opt_in() {
    let h = harness_with_database("report-rows");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);
    let report = h.root.join("report.md");

    let output = h.run(&[
        "investigation",
        "run",
        &id,
        "--report",
        report.to_str().unwrap(),
        "--rows",
        "1",
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let document = fs::read_to_string(&report).unwrap();
    assert!(document.contains("| id | label |"), "{document}");
    assert!(document.contains("| 1 | first |"), "{document}");
    assert!(
        !document.contains("| 2 | second |"),
        "only the requested rows land: {document}"
    );
    assert!(
        document.contains("Showing 1 of 2 captured rows."),
        "{document}"
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn stale_run_writes_no_report() {
    let h = harness_with_database("report-stale");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);
    let report = h.root.join("report.md");

    // The definition is revised on disk (revision 2, binding says 1).
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    fs::write(
        h.document_path(&id),
        document.replace("\"revision\": 1", "\"revision\": 2"),
    )
    .unwrap();

    let output = h.run(&[
        "investigation",
        "run",
        &id,
        "--report",
        report.to_str().unwrap(),
        "--rows",
        "2",
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", h.stderr(&output));
    assert!(
        h.stderr(&output).contains("revision changed"),
        "the refusal names what changed: {}",
        h.stderr(&output)
    );
    assert!(!report.exists(), "a refused run creates no report file");
    assert!(
        !h.stdout(&output).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&output)
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn run_report_refuses_existing_without_overwrite() {
    let h = harness_with_database("report-existing");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);
    let report = h.root.join("report.md");
    let report_arg = report.to_str().unwrap();

    let first = h.run(&["investigation", "run", &id, "--report", report_arg]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&first),
        h.stderr(&first)
    );
    // Mark the file so an accidental clobber is detectable.
    fs::write(&report, b"sentinel").unwrap();

    let refused = h.run(&["investigation", "run", &id, "--report", report_arg]);
    assert_eq!(refused.status.code(), Some(2), "{}", h.stderr(&refused));
    assert!(
        h.stderr(&refused).contains("exists; add --overwrite"),
        "the refusal names the way out: {}",
        h.stderr(&refused)
    );
    assert_eq!(
        fs::read(&report).unwrap(),
        b"sentinel",
        "the existing report is untouched"
    );
    assert!(
        h.binding_path(&id).exists(),
        "the S7 binding rules are unaffected by the report refusal"
    );

    let replaced = h.run(&[
        "investigation",
        "run",
        &id,
        "--report",
        report_arg,
        "--overwrite",
    ]);
    assert_eq!(
        replaced.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&replaced),
        h.stderr(&replaced)
    );
    assert!(
        fs::read_to_string(&report)
            .unwrap()
            .contains("# saya report"),
        "the report replaced the old file"
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn report_flags_without_report_are_usage_errors() {
    let h = harness_with_database("report-usage");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);
    let report = h.root.join("report.md");

    let rows_only = h.run(&["investigation", "run", &id, "--rows", "2"]);
    assert_eq!(rows_only.status.code(), Some(2), "{}", h.stderr(&rows_only));
    assert!(
        h.stderr(&rows_only).contains("--rows requires --report"),
        "{}",
        h.stderr(&rows_only)
    );

    let overwrite_only = h.run(&["investigation", "run", &id, "--overwrite"]);
    assert_eq!(
        overwrite_only.status.code(),
        Some(2),
        "{}",
        h.stderr(&overwrite_only)
    );
    assert!(
        h.stderr(&overwrite_only)
            .contains("--overwrite requires --report"),
        "{}",
        h.stderr(&overwrite_only)
    );

    let over_cap = h.run(&[
        "investigation",
        "run",
        &id,
        "--report",
        report.to_str().unwrap(),
        "--rows",
        "101",
    ]);
    assert_eq!(over_cap.status.code(), Some(2), "{}", h.stderr(&over_cap));
    assert!(
        h.stderr(&over_cap).contains("--rows is capped at 100"),
        "{}",
        h.stderr(&over_cap)
    );
    assert!(!report.exists(), "a usage error writes nothing");
    assert!(
        !h.stdout(&over_cap).contains("\"event\":\"query_result\""),
        "a usage error never executes: {}",
        h.stdout(&over_cap)
    );
    let _ = fs::remove_dir_all(h.root);
}
