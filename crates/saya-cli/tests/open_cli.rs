//! Integration tests for `saya open <file>` (C1b): one command from a local
//! CSV file to a read-only session over it. The file is staged once into a
//! private DuckDB snapshot keyed by its content hash (reused when the same
//! content opens again), a preview prints, and a generated read-only profile
//! runs through the normal CLI read path. Tests run the real binary against
//! isolated `SAYA_FILES_DIR`/`SAYA_CONFIG_HOME`/`SAYA_STATE_DB`/`HOME`.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use clap::Parser as _;
use saya_cli::Cli;
use sha2::{Digest, Sha256};

fn isolated_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-cli-open-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn open_command(root: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_saya"));
    cmd.env("SAYA_FILES_DIR", root.join("files"))
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", root);
    cmd
}

fn query_command(root: &Path, connections: &Path, profile: &str, sql: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_saya"));
    cmd.args(["--non-interactive", "--format", "json", "--connections"])
        .arg(connections)
        .args(["--profile", profile, "query", "--sql", sql])
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", root);
    cmd
}

/// The snapshot directories for `sha`: every entry under the files root whose
/// name carries the content-hash prefix (`<sha16>-<contract digest>`).
fn snapshot_dirs(root: &Path, sha: &str) -> Vec<PathBuf> {
    let prefix = &sha[..16];
    let Ok(entries) = fs::read_dir(root.join("files")) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix))
        })
        .collect();
    found.sort();
    found
}

/// The single snapshot directory for `sha` (panics when zero or several —
/// several means the content was opened under different parse options).
fn snapshot_dir(root: &Path, sha: &str) -> PathBuf {
    let dirs = snapshot_dirs(root, sha);
    assert_eq!(
        dirs.len(),
        1,
        "exactly one snapshot for content {}: {dirs:?}",
        &sha[..16]
    );
    dirs[0].clone()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The CLI grammar parses and the flags land on the `Open` variant. This test
/// also compiles against `Command::Open`, so the variant's shape is pinned.
#[test]
fn open_grammar_parses_and_pins_the_flags() {
    let parsed = Cli::try_parse_from([
        "saya",
        "open",
        "data/sales.csv",
        "--delimiter",
        ";",
        "--no-header",
        "--typed",
    ])
    .expect("open parses its grammar");
    match parsed.command {
        Some(saya_cli::Command::Open {
            file,
            delimiter,
            no_header,
            reset,
            typed,
            list,
            cleanup,
        }) => {
            assert_eq!(file.as_deref(), Some(Path::new("data/sales.csv")));
            assert_eq!(delimiter.as_deref(), Some(";"));
            assert!(no_header);
            assert!(!reset);
            assert!(typed);
            assert!(!list);
            assert!(cleanup.is_none());
        }
        other => panic!("expected Command::Open, got {other:?}"),
    }
    let listed = Cli::try_parse_from(["saya", "open", "--list"]).expect("--list parses");
    assert!(
        matches!(
            listed.command,
            Some(saya_cli::Command::Open { list: true, .. })
        ),
        "--list lands on the Open variant: {listed:?}"
    );
    let cleaned =
        Cli::try_parse_from(["saya", "open", "--cleanup", "abc123"]).expect("--cleanup parses");
    let Some(saya_cli::Command::Open {
        cleanup: Some(prefix),
        ..
    }) = cleaned.command
    else {
        panic!(
            "expected Command::Open with --cleanup, got {:?}",
            cleaned.command
        );
    };
    assert_eq!(prefix, "abc123");
}

#[test]
fn file_session_needs_no_database_profile() {
    let root = isolated_root("main");
    let csv = root.join("sales.csv");
    let content = "id,name,price\n1,widget,9.5\n2,gadget,19.99\n3,sprocket,\n";
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let out = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "open must succeed headless, stderr: {}",
        stderr_of(&out)
    );
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("File: sales.csv"),
        "names the file: {stdout}"
    );
    assert!(
        stdout.contains(&format!("SHA-256: {}", &sha[..12])),
        "shows the sha prefix: {stdout}"
    );
    assert!(stdout.contains("Rows: 3"), "shows the row count: {stdout}");
    assert!(
        stdout.contains("id: integer"),
        "shows inferred types: {stdout}"
    );
    assert!(
        stdout.contains("price: decimal (1 nulls)"),
        "shows null counts: {stdout}"
    );
    assert!(
        stdout.contains("Stored as text columns; use --typed for a typed copy."),
        "states the text storage: {stdout}"
    );
    assert!(
        stdout.contains("profile file_sales"),
        "names the profile: {stdout}"
    );

    let dir = snapshot_dir(&root, &sha);
    let db = dir.join("source.duckdb");
    assert!(
        db.exists(),
        "the staged db lands at <root>/<sha16>-<digest>/source.duckdb"
    );
    let mode = fs::metadata(&db).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "the staged db file is 0600");
    let dir_mode = fs::metadata(&dir).unwrap().permissions().mode();
    assert_eq!(dir_mode & 0o777, 0o700, "the snapshot directory is 0700");
    let connections = dir.join("connections.toml");
    assert!(connections.exists(), "connections.toml sits beside the db");

    let select = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales",
        "SELECT count(*) FROM sales",
    )
    .output()
    .unwrap();
    assert_eq!(
        select.status.code(),
        Some(0),
        "select must succeed through the generated profile, stderr: {}",
        stderr_of(&select)
    );
    assert!(
        stdout_of(&select).contains("\"event\":\"query_result\""),
        "select renders a query result: {}",
        stdout_of(&select)
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn open_one_file_does_not_grant_parent_access() {
    let root = isolated_root("parent");
    let content = "id,when\n1,2026-01-02\n2,2026-02-03\n";
    fs::write(root.join("sales.csv"), content).unwrap();
    fs::write(root.join("secret.txt"), "secret-value\n").unwrap();
    let sha = sha256_hex(content.as_bytes());

    let out = open_command(&root)
        .args([
            "open",
            root.join("sales.csv").to_str().unwrap(),
            "--non-interactive",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));

    let tables = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales",
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema NOT IN ('information_schema', 'pg_catalog') ORDER BY table_name",
    )
    .output()
    .unwrap();
    assert_eq!(tables.status.code(), Some(0), "{}", stderr_of(&tables));
    let stdout = stdout_of(&tables);
    assert!(
        stdout.contains("sales"),
        "the staged table is there: {stdout}"
    );
    assert!(
        stdout.contains("saya_file_source"),
        "the metadata table is there: {stdout}"
    );
    assert!(
        !stdout.contains("secret"),
        "nothing from the parent directory is reachable: {stdout}"
    );

    let probe = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales",
        &format!(
            "SELECT * FROM read_csv('{}')",
            root.join("secret.txt").display()
        ),
    )
    .output()
    .unwrap();
    assert_eq!(
        probe.status.code(),
        Some(4),
        "read_csv must be refused by the safety gate: {} {}",
        stdout_of(&probe),
        stderr_of(&probe)
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn leading_zero_ids_stay_text() {
    let root = isolated_root("leading-zero");
    let content = "id,qty\n007,3\n008,5\n";
    let csv = root.join("batch.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let out = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--typed",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("id: text"),
        "leading zeros stay text: {stdout}"
    );
    assert!(stdout.contains("qty: integer"), "{stdout}");
    assert!(
        stdout.contains("kept as text: id"),
        "the typed copy leaves text columns alone: {stdout}"
    );
    assert!(stdout.contains("Cast failures: none"), "{stdout}");

    let first = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_batch",
        "SELECT id FROM batch_typed ORDER BY id",
    )
    .output()
    .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    assert!(
        stdout_of(&first).contains("007"),
        "the typed copy keeps the leading zeros: {}",
        stdout_of(&first)
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn source_change_invalidates_previous_evidence() {
    let root = isolated_root("changed");
    let csv = root.join("ledger.csv");
    let first = "id,amount\n1,10\n";
    fs::write(&csv, first).unwrap();
    let sha_a = sha256_hex(first.as_bytes());
    let open_a = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(open_a.status.code(), Some(0), "{}", stderr_of(&open_a));

    let second = "id,amount\n1,10\n2,20\n";
    fs::write(&csv, second).unwrap();
    let sha_b = sha256_hex(second.as_bytes());
    let open_b = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(open_b.status.code(), Some(0), "{}", stderr_of(&open_b));

    let list = open_command(&root)
        .args(["open", "--list", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(list.status.code(), Some(0), "{}", stderr_of(&list));
    let stdout = stdout_of(&list);
    assert!(
        stdout.contains(&sha_a[..12]),
        "the old snapshot is still listed: {stdout}"
    );
    assert!(
        stdout.contains(&sha_b[..12]),
        "the changed content is a new snapshot: {stdout}"
    );
    assert!(
        snapshot_dirs(&root, &sha_a)
            .first()
            .is_some_and(|dir| dir.join("source.duckdb").exists()),
        "the old snapshot remains until cleanup"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn file_session_cleanup_removes_only_owned_state() {
    let root = isolated_root("cleanup");
    let content = "id\n1\n2\n";
    let csv = root.join("sales.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());
    let out = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));

    let foreign = root.join("files").join("not-a-saya-source");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("keep.txt"), "foreign state\n").unwrap();

    let cleanup = open_command(&root)
        .args(["open", "--cleanup", "all", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(cleanup.status.code(), Some(0), "{}", stderr_of(&cleanup));
    let stdout = stdout_of(&cleanup);
    assert!(
        stdout.contains("Removed:"),
        "cleanup prints what it removed: {stdout}"
    );
    assert!(stdout.contains(&sha[..12]), "{stdout}");
    assert!(
        snapshot_dirs(&root, &sha).is_empty(),
        "the staged source is removed"
    );
    assert!(
        foreign.join("keep.txt").exists(),
        "a foreign directory under the root survives cleanup --all"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn typed_table_reports_cast_failures() {
    let root = isolated_root("typed-failures");
    let mut content = String::from("n,label\n");
    for value in 1..=1000 {
        content.push_str(&format!("{value},ok\n"));
    }
    content.push_str("x,bad\n");
    let csv = root.join("metrics.csv");
    fs::write(&csv, &content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let out = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--typed",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("n: 1 of 1001 values did not cast"),
        "the cast failure is reported, never silently: {stdout}"
    );
    assert!(
        stdout.contains("kept NULL"),
        "the failure line says what happened to the values: {stdout}"
    );

    let count = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_metrics",
        "SELECT count(*) FROM metrics_typed",
    )
    .output()
    .unwrap();
    assert_eq!(count.status.code(), Some(0), "{}", stderr_of(&count));
    assert!(
        stdout_of(&count).contains("1001"),
        "the typed table has every row: {}",
        stdout_of(&count)
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn write_sql_on_the_file_profile_is_refused() {
    let root = isolated_root("refusal");
    let content = "id\n1\n2\n";
    let csv = root.join("sales.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());
    let out = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));

    let delete = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales",
        "DELETE FROM sales",
    )
    .output()
    .unwrap();
    assert_eq!(
        delete.status.code(),
        Some(4),
        "DELETE must be refused, stderr: {}",
        stderr_of(&delete)
    );
    assert!(
        stderr_of(&delete).contains("\"event\":\"error\""),
        "refusal renders an error event: {}",
        stderr_of(&delete)
    );
    let create = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales",
        "CREATE TABLE hack (a INTEGER)",
    )
    .output()
    .unwrap();
    assert_eq!(
        create.status.code(),
        Some(4),
        "CREATE must be refused, stderr: {}",
        stderr_of(&create)
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn same_content_reuses_the_staged_snapshot() {
    let root = isolated_root("reuse");
    let content = "id,name\n1,one\n";
    let csv = root.join("sales.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let first = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    let db = snapshot_dir(&root, &sha).join("source.duckdb");
    let before = fs::metadata(&db).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(50));

    let second = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0), "{}", stderr_of(&second));
    assert!(
        stdout_of(&second).contains("(reused existing snapshot)"),
        "the second open states the reuse: {}",
        stdout_of(&second)
    );
    assert_eq!(
        fs::metadata(&db).unwrap().modified().unwrap(),
        before,
        "reuse must not rewrite the staged snapshot"
    );
    let staged_count = fs::read_dir(root.join("files")).unwrap().count();
    assert_eq!(staged_count, 1, "reuse creates no second snapshot");

    let reset = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--reset",
        ])
        .output()
        .unwrap();
    assert_eq!(reset.status.code(), Some(0), "{}", stderr_of(&reset));
    assert_ne!(
        fs::metadata(&db).unwrap().modified().unwrap(),
        before,
        "--reset restages into the same snapshot directory"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn files_root_defaults_beside_the_state_db() {
    let root = isolated_root("default-files-root");
    let content = "id\n1\n";
    let csv = root.join("sales.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());
    let out = Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", &root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(
        snapshot_dirs(&root, &sha)
            .first()
            .is_some_and(|dir| dir.join("source.duckdb").exists()),
        "without SAYA_FILES_DIR the root is <state parent>/files"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn open_without_a_target_is_a_usage_error() {
    let root = isolated_root("usage");
    let bare = open_command(&root)
        .args(["open", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(
        bare.status.code(),
        Some(2),
        "nothing to do must be a usage error, stderr: {}",
        stderr_of(&bare)
    );
    let both = open_command(&root)
        .args(["open", "--list", "--cleanup", "all", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(
        both.status.code(),
        Some(2),
        "--list with --cleanup is a usage error, stderr: {}",
        stderr_of(&both)
    );
    let _ = fs::remove_dir_all(&root);
}

/// C2a: a local Parquet file stages through the bounded private decode into
/// the same snapshot layout, previews with `Format: parquet` and native
/// column types, queries through the generated read-only profile, and the
/// session's read-only gate refuses `read_parquet` (the staged snapshot must
/// never be able to open other files).
#[test]
fn open_stages_a_parquet_file_and_refuses_file_functions() {
    let root = isolated_root("parquet");
    let pq = root.join("sales_2024.parquet");
    {
        let conn = duckdb::Connection::open_in_memory().expect("fixture connection");
        conn.execute_batch(&format!(
            "COPY (SELECT 1 AS id, 'widget' AS name, 9.5 AS price \
             UNION ALL SELECT 2, 'gadget', 19.99 \
             UNION ALL SELECT 3, 'sprocket', NULL) TO '{}' (FORMAT parquet)",
            pq.display()
        ))
        .expect("fixture parquet");
    }
    let bytes = fs::read(&pq).unwrap();
    let sha = sha256_hex(&bytes);

    let out = open_command(&root)
        .args(["open", pq.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "open must succeed headless, stderr: {}",
        stderr_of(&out)
    );
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("File: sales_2024.parquet"),
        "names the file: {stdout}"
    );
    assert!(
        stdout.contains("Format: parquet"),
        "reports the parquet format: {stdout}"
    );
    assert!(stdout.contains("Rows: 3"), "shows the row count: {stdout}");
    assert!(
        stdout.contains("id: integer"),
        "shows the native type: {stdout}"
    );
    assert!(
        stdout.contains("price: decimal (1 nulls)"),
        "shows null counts: {stdout}"
    );
    assert!(
        !stdout.contains("Stored as text columns"),
        "parquet keeps native types, the CSV text-storage line does not apply: {stdout}"
    );
    assert!(
        stdout.contains("profile file_sales_2024"),
        "names the profile: {stdout}"
    );

    let dir = snapshot_dir(&root, &sha);
    let db = dir.join("source.duckdb");
    assert!(
        db.exists(),
        "the staged db lands at <root>/<sha16>-<digest>/source.duckdb"
    );
    let mode = fs::metadata(&db).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "the staged db file is 0600");
    let dir_mode = fs::metadata(&dir).unwrap().permissions().mode();
    assert_eq!(dir_mode & 0o777, 0o700, "the snapshot directory is 0700");
    assert!(
        dir.join("connections.toml").exists(),
        "connections.toml sits beside the db"
    );

    let select = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales_2024",
        "SELECT count(*) FROM sales_2024",
    )
    .output()
    .unwrap();
    assert_eq!(
        select.status.code(),
        Some(0),
        "select must succeed through the generated profile, stderr: {}",
        stderr_of(&select)
    );

    let refused = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales_2024",
        "SELECT * FROM read_parquet('/etc/passwd')",
    )
    .output()
    .unwrap();
    assert_eq!(
        refused.status.code(),
        Some(4),
        "read_parquet must be refused by the session's safety gate: {} {}",
        stdout_of(&refused),
        stderr_of(&refused)
    );
    assert!(
        (stdout_of(&refused) + &stderr_of(&refused)).contains("read_parquet"),
        "the refusal names the function"
    );
    let _ = fs::remove_dir_all(&root);
}

/// CSV-only flags on a Parquet file are a usage error, refused before (by
/// extension) or after (by PAR1 magic) staging — never silently ignored.
#[test]
fn open_parquet_refuses_csv_only_flags() {
    let root = isolated_root("parquet-flags");
    let pq = root.join("measurements.parquet");
    {
        let conn = duckdb::Connection::open_in_memory().expect("fixture connection");
        conn.execute_batch(&format!(
            "COPY (SELECT 1 AS reading) TO '{}' (FORMAT parquet)",
            pq.display()
        ))
        .expect("fixture parquet");
    }
    for flags in [
        vec!["--typed"],
        vec!["--delimiter", ";"],
        vec!["--no-header"],
    ] {
        let out = open_command(&root)
            .arg("open")
            .arg(pq.to_str().unwrap())
            .arg("--non-interactive")
            .args(&flags)
            .output()
            .unwrap();
        assert_ne!(
            out.status.code(),
            Some(0),
            "CSV-only flags {flags:?} must be refused on a Parquet file: {} {}",
            stdout_of(&out),
            stderr_of(&out)
        );
        assert!(
            (stdout_of(&out) + &stderr_of(&out)).contains("Parquet"),
            "the refusal names the format conflict: {} {}",
            stdout_of(&out),
            stderr_of(&out)
        );
    }
    // The snapshot still stages and reopens without the flags.
    let bytes = fs::read(&pq).unwrap();
    let sha = sha256_hex(&bytes);
    let out = open_command(&root)
        .args(["open", pq.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(
        snapshot_dirs(&root, &sha)
            .first()
            .is_some_and(|dir| dir.join("source.duckdb").exists()),
        "the snapshot is staged"
    );
    let _ = fs::remove_dir_all(&root);
}
