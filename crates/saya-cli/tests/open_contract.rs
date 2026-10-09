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

include!("open_contract/snapshot_restage.rs");

include!("open_contract/reuse_cleanup.rs");
