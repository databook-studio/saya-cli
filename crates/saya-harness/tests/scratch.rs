//! The `scratch_sql` battery (M4-2): the red tests for the amended ADR 0003
//! decision. The validator is the thing under test — every file-reading
//! statement is refused by it, by name, before the statement reaches DuckDB;
//! DuckDB's own permission-layer refusal is asserted separately, as the
//! backstop. The battery runs both directions: every statement `scratch_sql`
//! accepts stays inside the run dir (the sentinel planted outside is never
//! touched), and every write it accepts is still rejected by the user-DB
//! `prepare_*` functions, unmodified.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use proptest::prelude::*;
use saya_agent::{LocalStateEffect, ToolError, ToolExecutor};
use saya_connectors::{
    prepare_bigquery_sql, prepare_clickhouse_sql, prepare_duckdb_sql, prepare_mysql_sql,
    prepare_postgres_sql, prepare_snowflake_sql, prepare_sqlite_sql,
};
use saya_harness::{
    run_dir::RunDir,
    scratch::{
        MAX_SQL_BYTES, SCRATCH_FILE_NAME, ScratchError, ScratchRejection, ScratchSql, validate,
    },
};
use saya_types::{Capabilities, RunId};
use serde_json::{Value, json};

const SENTINEL_BYTES: &[u8] = b"sentinel-MUST-NOT-LEAK";

/// A run directory under a temp runs root, created by the production
/// [`RunDir::create`] (0700), plus a sentinel planted *outside* the run dir —
/// a real readable file any file-reading carve-out would find.
struct TempRun {
    runs_root: PathBuf,
    run_dir: PathBuf,
}

impl TempRun {
    fn new(tag: &str) -> Self {
        let runs_root =
            std::env::temp_dir().join(format!("saya-scratch-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&runs_root);
        fs::create_dir_all(&runs_root).unwrap();
        let id = RunId::parse("scratch-battery").unwrap();
        let dir = RunDir::create(&runs_root, &id).unwrap();
        Self {
            runs_root,
            run_dir: dir.root().to_path_buf(),
        }
    }

    fn root(&self) -> &Path {
        &self.run_dir
    }

    /// The sentinel: in the runs root, a sibling of the run dir — outside it.
    fn plant_sentinel(&self) -> PathBuf {
        let path = self.runs_root.join("sentinel.csv");
        fs::write(&path, SENTINEL_BYTES).unwrap();
        path
    }

    fn sentinel_untouched(&self, path: &Path) {
        assert_eq!(
            fs::read(path).unwrap(),
            SENTINEL_BYTES,
            "the sentinel outside the run dir must never be touched"
        );
    }

    fn scratch_file(&self) -> PathBuf {
        self.run_dir.join(SCRATCH_FILE_NAME)
    }
}

impl Drop for TempRun {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.runs_root);
    }
}

fn scratch_approved() -> Capabilities {
    let mut caps = Capabilities::default();
    caps.scratch = true;
    caps
}

#[test]
fn sql_over_the_preparse_bound_is_refused_before_sqlparser() {
    let sql = "x".repeat(MAX_SQL_BYTES + 1);
    let error = validate(&sql).expect_err("oversized SQL must be refused");
    assert!(matches!(error, ScratchRejection::TooLarge { .. }));
}

/// A019: the pre-parser admission boundary is exact. A payload of exactly
/// `MAX_SQL_BYTES` passes the byte gate (it may still fail parsing on its own
/// merits), while one byte more is refused as `TooLarge` without parsing. Both
/// go through `validate` — the admission boundary `ScratchSql::run` enters —
/// and both payloads are adversarial-shaped (unparsable `x` runs), so a pass
/// proves the byte gate admitted the input rather than the parser being cheap.
#[tokio::test]
async fn scratch_sql_admits_exactly_at_the_preparse_byte_bound() {
    let run = TempRun::new("exact-limit");
    let tool = admitted(&run);
    // Exactly at the limit: the byte gate admits; the payload itself is
    // unparsable, so the failure must come from the parser, not the gate.
    let sql = "x".repeat(MAX_SQL_BYTES);
    assert_eq!(sql.len(), MAX_SQL_BYTES);
    match tool.run(&sql).await {
        Err(ScratchError::Refused(ScratchRejection::Unparsable)) => {}
        other => panic!("at-limit input must reach the parser, got {other:?}"),
    }
    // One byte over: the byte gate refuses before the parser runs.
    let sql = "x".repeat(MAX_SQL_BYTES + 1);
    match tool.run(&sql).await {
        Err(ScratchError::Refused(ScratchRejection::TooLarge { bytes, max })) => {
            assert_eq!(bytes, MAX_SQL_BYTES + 1);
            assert_eq!(max, MAX_SQL_BYTES);
        }
        other => panic!("over-limit input must be refused pre-parse, got {other:?}"),
    }
    drop(tool);
    let _ = fs::remove_dir_all(&run.runs_root);
}

/// The rows of a [`QueryResult`]-shaped expectation, as a `Vec<Value>` for
/// direct comparison.
fn json_rows(rows: Value) -> Vec<Value> {
    rows.as_array().expect("row array").clone()
}

fn admitted(run: &TempRun) -> ScratchSql {
    ScratchSql::admit(run.root(), &scratch_approved())
        .unwrap()
        .expect("the scratch scope is approved")
}

/// Drives the async tool from the property battery's synchronous closure.
fn run_sync(tool: &ScratchSql, sql: &str) -> Result<saya_types::QueryResult, ScratchError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(tool.run(sql))
}

// ---------------------------------------------------------------------------
// 2. The file-reading family is refused by the validator, by name, before it
//    reaches DuckDB — and DuckDB refuses it too, asserted separately.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_file_reading_function_is_refused_by_the_validator_by_name_before_duckdb() {
    let run = TempRun::new("file-function");
    let tool = admitted(&run);
    for (sql, name) in [
        ("SELECT * FROM read_csv('x.csv')", "read_csv"),
        ("SELECT * FROM read_csv_auto('x.csv')", "read_csv_auto"),
        ("SELECT * FROM read_parquet('x.parquet')", "read_parquet"),
        ("SELECT * FROM read_json_auto('x.json')", "read_json_auto"),
        ("SELECT read_text('x.txt')", "read_text"),
        ("SELECT * FROM glob('*.csv')", "glob"),
        ("SELECT * FROM sqlite_scan('x.db', 't')", "sqlite_scan"),
        ("SELECT * FROM parquet_scan('x.parquet')", "parquet_scan"),
        ("SELECT http_get('http://127.0.0.1:1/x')", "http_get"),
        // Hidden behind a schema qualification and behind a write:
        ("SELECT * FROM main.read_csv('x.csv')", "read_csv"),
        ("INSERT INTO t SELECT * FROM read_csv('x.csv')", "read_csv"),
        (
            "CREATE TABLE c AS SELECT * FROM read_parquet('x.parquet')",
            "read_parquet",
        ),
    ] {
        match tool.run(sql).await {
            Err(ScratchError::Refused(ScratchRejection::FileFunction(refused))) => {
                assert_eq!(
                    refused, name,
                    "the refusal must name the file reader: {sql}"
                );
            }
            other => panic!("{sql} must be refused by the validator, got {other:?}"),
        }
    }
    // The refusals all happened before any statement reached DuckDB: nothing
    // was executed. Reopen only after the tool is dropped — the file lock
    // belongs to the tool's connection.
    drop(tool);
    let conn = reopened_scratch(&run);
    let tables = conn
        .query_row(
            "SELECT count(*) FROM information_schema.tables WHERE table_schema NOT IN ('information_schema','pg_catalog')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(
        tables, 0,
        "refused statements must not have created anything"
    );
}

#[tokio::test]
async fn a_file_reading_function_hidden_in_a_write_is_refused_before_duckdb() {
    let run = TempRun::new("hidden-in-write");
    let tool = admitted(&run);
    tool.run("CREATE TABLE stage(a INTEGER)").await.unwrap();
    match tool
        .run("INSERT INTO stage SELECT * FROM read_csv('x.csv')")
        .await
    {
        Err(ScratchError::Refused(ScratchRejection::FileFunction(name))) => {
            assert_eq!(name, "read_csv");
        }
        other => panic!("must be refused by the validator, got {other:?}"),
    }
    // The refused statement never reached the engine: the staging table is
    // still empty. Reopen only after the tool is dropped — the file lock
    // belongs to the tool's connection.
    drop(tool);
    let conn = reopened_scratch(&run);
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM stage", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

/// The backstop, asserted separately: DuckDB's own permission layer refuses
/// the same statements — install, load, an http read *and a local file read* —
/// with external access off. The flag is all-or-nothing; this is the second,
/// independent half of the belt-and-braces.
#[test]
fn duckdb_itself_refuses_file_reads_at_the_permission_layer() {
    let run = TempRun::new("backstop");
    let sentinel = run.plant_sentinel();
    let config = duckdb::Config::default()
        .access_mode(duckdb::AccessMode::ReadWrite)
        .and_then(|item| item.enable_external_access(false))
        .and_then(|item| item.enable_autoload_extension(false))
        .and_then(|item| item.with("allow_community_extensions", "false"))
        .and_then(|item| item.with("allow_persistent_secrets", "false"))
        .and_then(|item| item.with("lock_configuration", "true"))
        .expect("the pinned scratch configuration must build");
    let conn = duckdb::Connection::open_with_flags(run.scratch_file(), config).unwrap();
    for sql in [
        "INSTALL httpfs".to_string(),
        "LOAD httpfs".to_string(),
        format!("SELECT * FROM read_csv('{}')", sentinel.display()),
        format!("SELECT * FROM '{}'", sentinel.display()),
    ] {
        let error = conn
            .execute_batch(&sql)
            .expect_err("every file route must be refused")
            .to_string();
        assert!(
            error.contains("Permission Error"),
            "{sql} was not refused at the permission layer: {error}"
        );
    }
    run.sentinel_untouched(&sentinel);
}

// ---------------------------------------------------------------------------
// 3. INSTALL/LOAD refused by the validator.

#[tokio::test]
async fn install_and_load_are_refused_by_the_validator() {
    let run = TempRun::new("install-load");
    let tool = admitted(&run);
    for sql in ["INSTALL httpfs", "LOAD httpfs", "INSTALL json", "LOAD icu"] {
        match tool.run(sql).await {
            Err(ScratchError::Refused(ScratchRejection::Statement(kind))) => {
                assert!(
                    kind.starts_with(sql.split(' ').next().unwrap()),
                    "{sql}: {kind}"
                );
            }
            other => panic!("{sql} must be refused by the validator, got {other:?}"),
        }
    }
    run.sentinel_untouched(&run.plant_sentinel());
}

// ---------------------------------------------------------------------------
// 4. Multi-statement input refused.

#[tokio::test]
async fn multi_statement_input_is_refused() {
    let run = TempRun::new("multi-statement");
    let tool = admitted(&run);
    for sql in [
        "SELECT 1; SELECT 2",
        "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1)",
        "SELECT 1; INSTALL httpfs",
    ] {
        match tool.run(sql).await {
            Err(ScratchError::Refused(ScratchRejection::MultipleStatements { count })) => {
                assert_eq!(count, 2, "{sql}");
            }
            other => panic!("{sql} must be refused, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 5. The capability survives: CREATE TABLE / INSERT / SELECT round-trip.

#[tokio::test]
async fn create_insert_select_round_trip_works_and_reads_stay_capped() {
    let run = TempRun::new("round-trip");
    let tool = admitted(&run);

    let created = tool
        .run("CREATE TABLE predictions(id INTEGER, gold INTEGER, pred INTEGER)")
        .await
        .unwrap();
    assert_eq!(created.row_count, 0);

    let inserted = tool
        .run("INSERT INTO predictions VALUES (1, 10, 9), (2, 20, 20)")
        .await
        .unwrap();
    assert_eq!(inserted.row_count, 2);

    let selected = tool
        .run("SELECT id, gold, pred FROM predictions ORDER BY id")
        .await
        .unwrap();
    assert_eq!(selected.columns, ["id", "gold", "pred"]);
    assert_eq!(selected.rows, json_rows(json!([[1, 10, 9], [2, 20, 20]])));

    let joined = tool
        .run("SELECT count(*) AS misses FROM predictions WHERE gold != pred")
        .await
        .unwrap();
    assert_eq!(joined.rows, json_rows(json!([[1]])));

    // The 50-row discipline: 60 staged rows come back as exactly 50 with
    // `truncated` set, and the executed SQL is the statement the caller gave.
    let staged = tool
        .run("INSERT INTO predictions SELECT range, range, range FROM range(60)")
        .await
        .unwrap();
    assert_eq!(staged.row_count, 60);
    let sql = "SELECT id FROM predictions ORDER BY id";
    let capped = tool.run(sql).await.unwrap();
    assert_eq!(capped.rows.len(), 50);
    assert!(capped.truncated);
    assert_eq!(capped.executed_sql, sql);

    // An explicit bound at or under the cap is left as written.
    let bounded = tool
        .run("SELECT id FROM predictions LIMIT 5")
        .await
        .unwrap();
    assert_eq!(bounded.rows.len(), 5);
    assert!(!bounded.truncated);

    // UPDATE and DELETE stay available on the staged rows.
    let updated = tool
        .run("UPDATE predictions SET pred = gold WHERE id > 55")
        .await
        .unwrap();
    assert!(updated.row_count > 0);
    let deleted = tool
        .run("DELETE FROM predictions WHERE id > 40")
        .await
        .unwrap();
    assert!(deleted.row_count > 0);

    // SHOW still reads.
    let shown = tool.run("SHOW TABLES").await.unwrap();
    assert_eq!(shown.row_count, 1);
}

// ---------------------------------------------------------------------------
// 6. Structural: scratch never travels the DatabaseConnector path.

/// No type-level path may run from a scratch write to a user database. The
/// proof is structural, in two halves:
///
/// 1. The harness crate cannot even *name* the connector crate: its shipped
///    `[dependencies]` never list `saya-connectors`, so no type in this crate
///    can implement `DatabaseConnector`, and the `ConnectionRegistry` — which
///    stores `Box<dyn DatabaseConnector>` — cannot hold one.
/// 2. The scratch module's *code* never names the types it would need to fake
///    it, guarding against a future dependency being slipped in quietly. The
///    scan strips comments so honest docs about the separation are fine; only
///    code can trip it.
#[test]
fn scratch_has_no_type_level_path_to_user_databases() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    let shipped = manifest_section(&manifest, "[dependencies]");
    assert!(
        !shipped.contains("saya-connectors"),
        "saya-harness must not depend on the connector crate: scratch has no \
         connector path by construction"
    );

    let scratch_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/scratch");
    for entry in fs::read_dir(&scratch_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let code = code_only(&fs::read_to_string(&path).unwrap());
        for forbidden in [
            "DatabaseConnector",
            "ConnectionRegistry",
            "ConnectionEntry",
            "saya_connectors",
        ] {
            assert!(
                !code.contains(forbidden),
                "{} names {forbidden}: the scratch surface must never reference \
                 the user-database connector path",
                path.display()
            );
        }
    }
}

/// The source with line comments (`//`, `//!`, `///`) stripped, so the scan
/// sees only code.
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn manifest_section(manifest: &str, header: &str) -> String {
    let start = manifest.find(header).map(|i| i + header.len()).unwrap_or(0);
    let end = manifest[start..]
        .find("\n[")
        .map(|i| start + i)
        .unwrap_or(manifest.len());
    manifest[start..end].to_string()
}

// ---------------------------------------------------------------------------
// 7. The created file is 0600 after open.

#[cfg(unix)]
#[test]
fn the_created_scratch_file_is_0600() {
    use std::os::unix::fs::PermissionsExt;
    let run = TempRun::new("file-mode");
    assert!(!run.scratch_file().exists());
    let _tool = admitted(&run);
    let mode = fs::metadata(run.scratch_file())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "DuckDB creates 0644; the engine does not rely on the run directory alone"
    );
}

// ---------------------------------------------------------------------------
// Admission: hidden-not-advertised until the scratch scope is approved.

#[test]
fn the_tool_is_hidden_until_the_scratch_scope_is_approved() {
    let run = TempRun::new("admission");
    assert!(
        ScratchSql::definitions(&Capabilities::default()).is_empty(),
        "without the scope the tool is hidden, never advertised as always-empty"
    );
    assert!(
        ScratchSql::admit(run.root(), &Capabilities::default())
            .unwrap()
            .is_none(),
        "without the scope the tool does not exist"
    );
    assert!(
        !run.scratch_file().exists(),
        "a refused admission opens nothing"
    );

    let caps = scratch_approved();
    let definitions = ScratchSql::definitions(&caps);
    assert_eq!(definitions.len(), 1);
    let definition = &definitions[0];
    assert_eq!(definition.name, "scratch_sql");
    assert!(!definition.read_only, "the write is the honest surface");
    assert!(
        !definition.effect.requires_approval,
        "the write is plan-gated, not per-call approved"
    );
    assert_eq!(
        definition.effect.local_state,
        LocalStateEffect::WriteWorkspace,
        "the declared effect is write-shaped local state"
    );

    let _tool = ScratchSql::admit(run.root(), &caps).unwrap().unwrap();
    assert!(run.scratch_file().exists(), "the approved admission opens");
}

#[test]
fn the_tool_executor_refuses_bad_arguments_with_typed_errors() {
    let run = TempRun::new("executor-args");
    let tool = admitted(&run);
    let runner = |name: &str, args: Value| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(tool.execute(name, args))
    };
    assert!(matches!(
        runner("other", json!({"sql": "SELECT 1"})),
        Err(ToolError::UnsupportedTool)
    ));
    assert!(matches!(
        runner("scratch_sql", json!("SELECT 1")),
        Err(ToolError::ArgumentsNotObject)
    ));
    assert!(matches!(
        runner("scratch_sql", json!({})),
        Err(ToolError::UnsupportedProperty)
    ));
    assert!(matches!(
        runner("scratch_sql", json!({"sql": "SELECT 1", "extra": 1})),
        Err(ToolError::UnsupportedProperty)
    ));
    assert!(matches!(
        runner("scratch_sql", json!({"sql": 7})),
        Err(ToolError::SqlNotString)
    ));
    let ok = runner("scratch_sql", json!({"sql": "SELECT 42 AS answer"})).unwrap();
    assert_eq!(ok["rows"], json!([[42]]));
    let refused = runner("scratch_sql", json!({"sql": "INSTALL httpfs"})).unwrap_err();
    assert!(
        matches!(refused, ToolError::QueryFailedDetail(_)),
        "a refusal surfaces through the tool boundary: {refused}"
    );
    assert!(refused.to_string().contains("INSTALL httpfs"));
}

// ---------------------------------------------------------------------------
// Timeout plus interrupt-then-await: exactly the connector's shape.

#[tokio::test]
async fn a_runaway_statement_is_interrupted_and_the_connection_is_released() {
    let run = TempRun::new("timeout");
    let tool = admitted(&run).with_query_timeout(Duration::from_millis(500));
    let error = tool
        .run("SELECT count(*) FROM range(100000000) a, range(100000000) b")
        .await
        .expect_err("the cross product never finishes; the ceiling must fire");
    assert!(matches!(error, ScratchError::TimedOut), "{error}");
    // The await-on-cancel proved the query released the mutex: the connection
    // still serves.
    let answer = tool.run("SELECT 42 AS answer").await.unwrap();
    assert_eq!(answer.rows, json_rows(json!([[42]])));
}

// ---------------------------------------------------------------------------
// 1. The property battery, both directions.

/// What the corpus entry claims about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Shape {
    /// A write on the scratch file itself — accepted by scratch, and still
    /// rejected by every user-DB `prepare_*`.
    Write,
    /// A read — accepted by scratch.
    Read,
    /// Refused by the validator: a file reader (with the sentinel's real path
    /// where the statement takes one).
    RefusedFile,
    /// Refused by the validator: the file/configuration/extension family.
    RefusedKind,
    /// Refused by the validator: more than one statement.
    RefusedMulti,
}

fn corpus() -> Vec<(String, Shape)> {
    let entries = vec![
        // Writes — the capability.
        (
            "CREATE TABLE t(a INTEGER, b VARCHAR)".to_string(),
            Shape::Write,
        ),
        (
            "INSERT INTO t VALUES (1, 'alpha')".to_string(),
            Shape::Write,
        ),
        ("UPDATE t SET a = 2".to_string(), Shape::Write),
        ("DELETE FROM t".to_string(), Shape::Write),
        (
            "ALTER TABLE t ADD COLUMN c INTEGER".to_string(),
            Shape::Write,
        ),
        ("DROP TABLE t".to_string(), Shape::Write),
        ("TRUNCATE t".to_string(), Shape::Write),
        ("CREATE VIEW v AS SELECT 1 AS x".to_string(), Shape::Write),
        ("DROP VIEW v".to_string(), Shape::Write),
        ("CREATE SCHEMA s".to_string(), Shape::Write),
        ("DROP SCHEMA s".to_string(), Shape::Write),
        ("CREATE INDEX idx ON t(a)".to_string(), Shape::Write),
        // A write that also hands rows back — still a write to the user gate.
        (
            "INSERT INTO t VALUES (1) RETURNING a".to_string(),
            Shape::Write,
        ),
        // Reads.
        ("SELECT 1".to_string(), Shape::Read),
        ("SELECT a, b FROM t ORDER BY a".to_string(), Shape::Read),
        ("SELECT count(*) FROM t".to_string(), Shape::Read),
        (
            "WITH c AS (SELECT 1 AS x) SELECT x FROM c".to_string(),
            Shape::Read,
        ),
        ("SELECT 1 UNION ALL SELECT 2".to_string(), Shape::Read),
        ("EXPLAIN SELECT a FROM t".to_string(), Shape::Read),
        ("EXPLAIN ANALYZE SELECT * FROM t".to_string(), Shape::Read),
        ("SHOW TABLES".to_string(), Shape::Read),
        // Refused: the file-reading family, by name.
        (
            "SELECT * FROM read_csv('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "SELECT * FROM read_csv_auto('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "SELECT * FROM read_parquet('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "SELECT * FROM read_json('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        ("SELECT read_text('{SENT}')".to_string(), Shape::RefusedFile),
        (
            "SELECT * FROM glob('*.csv')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "SELECT * FROM sqlite_scan('{SENT}', 't')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "SELECT * FROM parquet_scan('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "SELECT http_get('http://127.0.0.1:1/x')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "INSERT INTO t SELECT * FROM read_csv('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "CREATE TABLE c AS SELECT * FROM read_parquet('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        // The file-read shorthand.
        ("SELECT * FROM '{SENT}'".to_string(), Shape::RefusedFile),
        // Refused statement kinds.
        ("INSTALL httpfs".to_string(), Shape::RefusedKind),
        ("LOAD httpfs".to_string(), Shape::RefusedKind),
        ("ATTACH '{SENT}' AS s".to_string(), Shape::RefusedKind),
        ("DETACH s".to_string(), Shape::RefusedKind),
        ("COPY t TO '{SENT}'".to_string(), Shape::RefusedKind),
        ("COPY t FROM '{SENT}'".to_string(), Shape::RefusedKind),
        (
            "PRAGMA enable_external_access".to_string(),
            Shape::RefusedKind,
        ),
        (
            "SET enable_external_access = true".to_string(),
            Shape::RefusedKind,
        ),
        ("SET threads = 2".to_string(), Shape::RefusedKind),
        ("CALL dbgen(sf = 1)".to_string(), Shape::RefusedKind),
        // Multi-statement.
        ("SELECT 1; SELECT 2".to_string(), Shape::RefusedMulti),
        (
            "CREATE TABLE u(a INTEGER); INSERT INTO u VALUES (1)".to_string(),
            Shape::RefusedMulti,
        ),
    ];
    entries
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Both directions at once. For every generated statement, executed
    /// through `scratch_sql` against a real run's scratch database:
    ///
    /// - accepted statements only ever touch the run dir — the sentinel
    ///   planted outside it keeps its exact bytes, and no accepted statement
    ///   ever trips DuckDB's permission layer;
    /// - every accepted *write* is still rejected by all seven user-DB
    ///   `prepare_*` functions, called exactly as they exist;
    /// - refused statements are refused by the validator with the typed
    ///   refusal their shape demands.
    #[test]
    fn every_accepted_statement_stays_inside_the_run_dir_and_every_write_is_rejected_by_the_user_gate(
        picked in proptest::collection::vec(proptest::sample::select(corpus()), 1..8),
    ) {
        let run = TempRun::new("battery");
        let sentinel = run.plant_sentinel();
        let tool = admitted(&run);
        for (template, shape) in picked {
            let sql = template.replace("{SENT}", &sentinel.display().to_string());
            match shape {
                Shape::Write => match run_sync(&tool, &sql) {
                    Ok(_) => {}
                    Err(ScratchError::Execution { message }) => prop_assert!(
                        !message.contains("Permission Error"),
                        "an accepted statement never trips the permission layer: {sql} — {message}"
                    ),
                    Err(other) => panic!("unexpected outcome for {sql}: {other}"),
                },
                Shape::Read => match run_sync(&tool, &sql) {
                    Ok(_) => {}
                    Err(ScratchError::Execution { message }) => prop_assert!(
                        !message.contains("Permission Error"),
                        "an accepted statement never trips the permission layer: {sql} — {message}"
                    ),
                    Err(other) => panic!("unexpected outcome for {sql}: {other}"),
                },
                Shape::RefusedFile => match run_sync(&tool, &sql) {
                    Err(ScratchError::Refused(ScratchRejection::FileFunction(_)))
                    | Err(ScratchError::Refused(ScratchRejection::FilePathRelation(_))) => {}
                    other => prop_assert!(
                        false,
                        "file reader must be refused by the validator: {other:?}"
                    ),
                },
                Shape::RefusedKind => match run_sync(&tool, &sql) {
                    Err(ScratchError::Refused(ScratchRejection::Statement(_))) => {}
                    other => prop_assert!(
                        false,
                        "statement family must be refused by the validator: {other:?}"
                    ),
                },
                Shape::RefusedMulti => match run_sync(&tool, &sql) {
                    Err(ScratchError::Refused(ScratchRejection::MultipleStatements {
                        count: 2,
                    })) => {}
                    other => prop_assert!(false, "multi-statement must be refused: {other:?}"),
                },
            }
            if shape == Shape::Write {
                for prepare in [
                    prepare_postgres_sql,
                    prepare_mysql_sql,
                    prepare_duckdb_sql,
                    prepare_snowflake_sql,
                    prepare_sqlite_sql,
                    prepare_clickhouse_sql,
                    prepare_bigquery_sql,
                ] {
                    prop_assert!(
                        prepare(&sql, 10).is_err(),
                        "the user-DB gate must still reject {sql}"
                    );
                }
            }
            run.sentinel_untouched(&sentinel);
        }
    }
}

/// Reopens the scratch file DuckDB-side (after the tool closed it) to inspect
/// what the engine actually holds.
fn reopened_scratch(run: &TempRun) -> duckdb::Connection {
    let config = duckdb::Config::default()
        .access_mode(duckdb::AccessMode::ReadWrite)
        .and_then(|item| item.enable_external_access(false))
        .and_then(|item| item.enable_autoload_extension(false))
        .and_then(|item| item.with("allow_community_extensions", "false"))
        .and_then(|item| item.with("allow_persistent_secrets", "false"))
        .and_then(|item| item.with("lock_configuration", "true"))
        .expect("the pinned scratch configuration must build");
    duckdb::Connection::open_with_flags(run.scratch_file(), config).unwrap()
}
