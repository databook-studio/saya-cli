//! Integration tests for the `saya open` snapshot identity (D4, A922-4): a
//! snapshot is reused only when the content hash AND the effective parse
//! contract (delimiter, header flag, table name, typed flag) match. Changed
//! options stage a separate snapshot of the same content, and the preview a
//! run prints is exactly what `saya query` returns through that run's printed
//! profile — both directions read the snapshot's own stored metadata.
//!
//! Tests run the real binary against isolated
//! `SAYA_FILES_DIR`/`SAYA_CONFIG_HOME`/`SAYA_STATE_DB`/`HOME`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;

fn isolated_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-cli-open-contract-{label}-{}",
        std::process::id()
    ));
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

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The connections path the preview's launch line names:
/// `  saya --connections <path> --profile <profile>`.
fn preview_connections(stdout: &str) -> PathBuf {
    let line = stdout
        .lines()
        .find(|line| line.contains("--connections"))
        .unwrap_or_else(|| panic!("preview names its launch command: {stdout}"));
    let rest = line
        .split("--connections ")
        .nth(1)
        .expect("launch line shape");
    PathBuf::from(rest.split_whitespace().next().expect("connections path"))
}

/// The profile the preview's launch line names: `file_<table>`.
fn preview_profile(stdout: &str) -> String {
    let line = stdout
        .lines()
        .find(|line| line.contains("--profile"))
        .unwrap_or_else(|| panic!("preview names its profile: {stdout}"));
    line.split("--profile ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("profile name")
        .to_owned()
}

/// The row count the preview claims: `Rows: <n> · Columns: <m>`.
fn preview_rows(stdout: &str) -> usize {
    let line = stdout
        .lines()
        .find(|line| line.starts_with("Rows: "))
        .unwrap_or_else(|| panic!("preview shows the row count: {stdout}"));
    line["Rows: ".len()..]
        .split(" · ")
        .next()
        .and_then(|rows| rows.trim().parse().ok())
        .unwrap_or_else(|| panic!("preview row count parses: {stdout}"))
}

/// The column names the preview claims, in order: the `  <name>: <type>
/// (<n> nulls)` lines between `Columns:` and the next blank section.
fn preview_columns(stdout: &str) -> Vec<String> {
    let mut columns = Vec::new();
    let mut in_columns = false;
    for line in stdout.lines() {
        if line == "Columns:" {
            in_columns = true;
        } else if in_columns {
            if !line.starts_with("  ") {
                break;
            }
            columns.push(
                line.trim()
                    .split(':')
                    .next()
                    .expect("column line has a name")
                    .to_owned(),
            );
        }
    }
    columns
}

/// Runs one query through a generated file profile and returns the parsed
/// `query_result` event (columns, rows, row_count).
fn query_result(root: &Path, connections: &Path, profile: &str, sql: &str) -> Value {
    let output = query_command(root, connections, profile, sql)
        .output()
        .expect("query runs");
    assert_eq!(
        output.status.code(),
        Some(0),
        "query must succeed: {} {}",
        stdout_of(&output),
        stderr_of(&output)
    );
    serde_json::from_str(stdout_of(&output).trim())
        .unwrap_or_else(|error| panic!("query renders a json event: {error}"))
}

/// A column probe that must fail: the named column is not in the snapshot's
/// table, so the query refuses.
fn column_query_fails(root: &Path, connections: &Path, profile: &str, sql: &str) {
    let output = query_command(root, connections, profile, sql)
        .output()
        .expect("probe runs");
    assert_ne!(
        output.status.code(),
        Some(0),
        "the probe must fail for {sql}: {} {}",
        stdout_of(&output),
        stderr_of(&output)
    );
}

/// Asserts the preview's claimed shape (rows, columns) equals what
/// `saya query` returns through the preview's own printed profile.
fn assert_preview_matches_query(root: &Path, stdout: &str, table: &str) {
    let connections = preview_connections(stdout);
    let profile = preview_profile(stdout);
    assert_eq!(profile, format!("file_{table}"), "profile names the table");
    let rows = preview_rows(stdout);
    let columns = preview_columns(stdout);
    let count = query_result(
        root,
        &connections,
        &profile,
        &format!("SELECT count(*) AS n FROM {table}"),
    );
    assert_eq!(
        count["result"]["row_count"].as_u64(),
        Some(1),
        "the count returns one row: {count}"
    );
    assert_eq!(
        count["result"]["rows"][0][0].as_u64(),
        Some(rows as u64),
        "the query's row count equals the preview's: {count} vs preview {rows}"
    );
    let shape = query_result(
        root,
        &connections,
        &profile,
        &format!("SELECT * FROM {table} LIMIT 0"),
    );
    let actual_columns = shape["result"]["columns"]
        .as_array()
        .expect("columns array")
        .iter()
        .map(|value| value.as_str().expect("column name").to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        actual_columns, columns,
        "the table's columns equal the preview's"
    );
}

/// The snapshot directory names of every staged snapshot under `root` whose
/// directory name carries `sha`'s 16-hex content prefix (the name is
/// `<sha16>-<contract digest>`), sorted.
fn snapshot_dirs(root: &Path, sha: &str) -> Vec<PathBuf> {
    let prefix = &sha[..16];
    let mut found: Vec<PathBuf> = match fs::read_dir(root.join("files")) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(prefix))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    found.sort();
    found
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A922-4 reproduction: opening the same content with `--no-header` must not
/// reuse the header snapshot — the second preview (2 rows, column_1/column_2)
/// must equal what a query through the second snapshot's profile returns.
#[test]
fn no_header_change_stages_a_separate_snapshot_whose_preview_matches_the_query() {
    let root = isolated_root("no-header");
    let content = "zip,city\n02134,Boston\n";
    let csv = root.join("a.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let first = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    let first_out = stdout_of(&first);
    assert!(first_out.contains("Rows: 1"), "{first_out}");
    assert!(first_out.contains("zip: text"), "{first_out}");
    assert!(first_out.contains("city: text"), "{first_out}");

    let second = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--no-header",
        ])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0), "{}", stderr_of(&second));
    let second_out = stdout_of(&second);
    assert!(
        second_out.contains("(staged now)"),
        "a changed header flag must stage a fresh snapshot, not reuse: {second_out}"
    );
    assert!(second_out.contains("Rows: 2"), "{second_out}");
    assert!(second_out.contains("column_1: text"), "{second_out}");
    assert!(second_out.contains("column_2: text"), "{second_out}");

    let dirs = snapshot_dirs(&root, &sha);
    assert_eq!(
        dirs.len(),
        2,
        "two snapshots of the same content: {:?}",
        dirs
    );
    for dir in &dirs {
        let name = dir.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with(&sha[..16]),
            "the name keeps the content-hash prefix first: {name}"
        );
    }
    assert_preview_matches_query(&root, &first_out, "a");
    assert_preview_matches_query(&root, &second_out, "a");
    // The wrong spelling fails in each direction: the header snapshot has no
    // column_1, the no-header one has no zip.
    let (first_connections, first_profile) =
        (preview_connections(&first_out), preview_profile(&first_out));
    let (second_connections, second_profile) = (
        preview_connections(&second_out),
        preview_profile(&second_out),
    );
    column_query_fails(
        &root,
        &first_connections,
        &first_profile,
        "SELECT column_1 FROM a LIMIT 1",
    );
    column_query_fails(
        &root,
        &second_connections,
        &second_profile,
        "SELECT zip FROM a LIMIT 1",
    );
    let _ = fs::remove_dir_all(&root);
}

/// A changed delimiter is a different contract: its own snapshot, and the
/// preview equals the query through that snapshot's profile.
#[test]
fn delimiter_change_stages_a_separate_snapshot_whose_preview_matches_the_query() {
    let root = isolated_root("delimiter");
    let content = "x;y\n1;2\n";
    let csv = root.join("d.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let first = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--delimiter",
            ";",
        ])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    let first_out = stdout_of(&first);
    assert!(first_out.contains("Rows: 1"), "{first_out}");
    assert!(first_out.contains("x: integer"), "{first_out}");

    let second = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--delimiter",
            ",",
            "--no-header",
        ])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0), "{}", stderr_of(&second));
    let second_out = stdout_of(&second);
    assert!(
        second_out.contains("(staged now)"),
        "a changed delimiter must stage a fresh snapshot, not reuse: {second_out}"
    );
    assert!(second_out.contains("Rows: 2"), "{second_out}");
    assert!(second_out.contains("column_1: text"), "{second_out}");

    let dirs = snapshot_dirs(&root, &sha);
    assert_eq!(
        dirs.len(),
        2,
        "two snapshots of the same content: {:?}",
        dirs
    );
    assert_preview_matches_query(&root, &first_out, "d");
    assert_preview_matches_query(&root, &second_out, "d");
    let _ = fs::remove_dir_all(&root);
}

/// The audit's renamed-file repro: identical content under a new name opened
/// with `--typed` stages its own snapshot (different table name and typed
/// flag in the contract) and the typed copy works — instead of the old
/// exit-2 `could not build the typed copy "b_typed"` against a foreign table.
#[test]
fn renamed_identical_file_with_typed_stages_its_own_snapshot() {
    let root = isolated_root("renamed-typed");
    let content = "zip,city\n02134,Boston\n";
    let a = root.join("a.csv");
    fs::write(&a, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let first = open_command(&root)
        .args(["open", a.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    let first_out = stdout_of(&first);

    let b = root.join("b.csv");
    fs::copy(&a, &b).unwrap();
    let second = open_command(&root)
        .args(["open", b.to_str().unwrap(), "--non-interactive", "--typed"])
        .output()
        .unwrap();
    assert_eq!(
        second.status.code(),
        Some(0),
        "the renamed --typed open must succeed: {} {}",
        stdout_of(&second),
        stderr_of(&second)
    );
    let second_out = stdout_of(&second);
    assert!(second_out.contains("File: b.csv"), "{second_out}");
    assert!(second_out.contains("Typed copy: b_typed"), "{second_out}");

    assert_eq!(
        snapshot_dirs(&root, &sha).len(),
        2,
        "table name and typed flag separate the snapshots"
    );
    assert_preview_matches_query(&root, &first_out, "a");
    assert_preview_matches_query(&root, &second_out, "b");
    let b_connections = preview_connections(&second_out);
    let typed = query_result(
        &root,
        &b_connections,
        "file_b",
        "SELECT zip FROM b_typed LIMIT 1",
    );
    assert_eq!(
        typed["result"]["rows"][0][0].as_str(),
        Some("02134"),
        "the typed copy holds the staged values: {typed}"
    );
    let _ = fs::remove_dir_all(&root);
}

/// Plain reuse survives: an identical invocation — and one whose explicit
/// delimiter only names the delimiter the sniff would pick anyway — reuses
/// the snapshot without restaging it.
#[test]
fn identical_invocation_still_reuses_without_restage() {
    let root = isolated_root("reuse");
    let content = "x;y\n1;2\n";
    let csv = root.join("r.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let first = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    let first_out = stdout_of(&first);
    assert!(
        first_out.contains("(staged now)"),
        "the first open stages: {first_out}"
    );
    let connections = preview_connections(&first_out);
    let db = connections.parent().unwrap().join("source.duckdb");
    let before = fs::metadata(&db).unwrap().modified().unwrap();

    let second = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0), "{}", stderr_of(&second));
    assert!(
        stdout_of(&second).contains("(reused existing snapshot)"),
        "the identical invocation reuses: {}",
        stdout_of(&second)
    );
    assert_eq!(
        snapshot_dirs(&root, &sha).len(),
        1,
        "reuse creates no second snapshot"
    );

    // An explicit delimiter that names what the sniff would pick resolves to
    // the same effective contract, so it reuses too.
    let third = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--delimiter",
            ";",
        ])
        .output()
        .unwrap();
    assert_eq!(third.status.code(), Some(0), "{}", stderr_of(&third));
    assert!(
        stdout_of(&third).contains("(reused existing snapshot)"),
        "the effective contract matches, so it reuses: {}",
        stdout_of(&third)
    );
    assert_eq!(
        fs::metadata(&db).unwrap().modified().unwrap(),
        before,
        "reuse must not rewrite the staged snapshot"
    );
    let _ = fs::remove_dir_all(&root);
}

/// `--cleanup <prefix>` over two snapshots of the same content is refused as
/// ambiguous; `all` removes both.
#[test]
fn cleanup_prefix_over_two_snapshots_of_one_content_is_refused_and_all_removes_both() {
    let root = isolated_root("cleanup-ambiguous");
    let content = "zip,city\n02134,Boston\n";
    let csv = root.join("a.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    for flags in [
        vec!["--non-interactive"],
        vec!["--non-interactive", "--no-header"],
    ] {
        let out = open_command(&root)
            .arg("open")
            .arg(csv.to_str().unwrap())
            .args(&flags)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    }
    assert_eq!(snapshot_dirs(&root, &sha).len(), 2);

    let refused = open_command(&root)
        .args(["open", "--cleanup", &sha[..12], "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(
        refused.status.code(),
        Some(2),
        "a prefix matching two snapshots of one content is ambiguous: {} {}",
        stdout_of(&refused),
        stderr_of(&refused)
    );
    assert!(
        stderr_of(&refused).contains("matches 2 staged sources"),
        "the refusal names the ambiguity: {}",
        stderr_of(&refused)
    );
    assert!(
        stderr_of(&refused).contains("differ only in parse options"),
        "same-content matches cannot be lengthened apart, so the refusal says so: {}",
        stderr_of(&refused)
    );
    assert_eq!(
        snapshot_dirs(&root, &sha).len(),
        2,
        "the ambiguous cleanup removes nothing"
    );

    let all = open_command(&root)
        .args(["open", "--cleanup", "all", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(all.status.code(), Some(0), "{}", stderr_of(&all));
    let all_out = stdout_of(&all);
    assert_eq!(
        all_out.matches("Removed:").count(),
        2,
        "`all` removes both snapshots: {all_out}"
    );
    assert!(
        snapshot_dirs(&root, &sha).is_empty(),
        "both snapshots are gone"
    );
    let _ = fs::remove_dir_all(&root);
}

/// A legacy snapshot (content-hash-only directory, no stored contract) is
/// still recognized: `--list` shows it, a matching open never reuses or
/// mutates it (it stages a fresh contract-suffixed snapshot), and
/// `--cleanup all` removes it too.
#[test]
fn legacy_snapshot_is_listed_never_reused_and_still_cleaned() {
    let root = isolated_root("legacy");
    let content = "id\n1\n";
    let csv = root.join("legacy.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    // Hand-build the pre-contract snapshot layout: a 16-hex directory whose
    // metadata table carries only the original seven keys.
    let legacy = root.join("files").join(&sha[..16]);
    fs::create_dir_all(&legacy).unwrap();
    {
        let connection = duckdb::Connection::open(legacy.join("source.duckdb")).unwrap();
        connection
            .execute_batch(&format!(
                "CREATE TABLE legacy (id VARCHAR); \
                 INSERT INTO legacy VALUES ('1'); \
                 CREATE TABLE saya_file_source (key VARCHAR, value VARCHAR); \
                 INSERT INTO saya_file_source VALUES \
                 ('file_name', 'legacy.csv'), ('sha256', '{sha}'), ('bytes', '6'), \
                 ('rows', '1'), ('columns', '1'), ('staged_unix_ms', '0'), ('format', 'csv');"
            ))
            .unwrap();
    }
    let legacy_db = legacy.join("source.duckdb");
    let before = fs::metadata(&legacy_db).unwrap().modified().unwrap();

    let list = open_command(&root)
        .args(["open", "--list", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(list.status.code(), Some(0), "{}", stderr_of(&list));
    assert!(
        stdout_of(&list).contains(&sha[..12]),
        "the legacy snapshot is listed: {}",
        stdout_of(&list)
    );

    let opened = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(opened.status.code(), Some(0), "{}", stderr_of(&opened));
    assert!(
        stdout_of(&opened).contains("(staged now)"),
        "a legacy snapshot has no stored contract, so it is never reused: {}",
        stdout_of(&opened)
    );
    assert_eq!(
        fs::metadata(&legacy_db).unwrap().modified().unwrap(),
        before,
        "the legacy snapshot is never mutated"
    );
    assert_eq!(
        snapshot_dirs(&root, &sha).len(),
        2,
        "the fresh contract-suffixed snapshot sits beside the legacy one"
    );

    let all = open_command(&root)
        .args(["open", "--cleanup", "all", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(all.status.code(), Some(0), "{}", stderr_of(&all));
    assert_eq!(
        stdout_of(&all).matches("Removed:").count(),
        2,
        "cleanup still reaches the legacy snapshot: {}",
        stdout_of(&all)
    );
    let _ = fs::remove_dir_all(&root);
}
