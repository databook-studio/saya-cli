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

include!("open_cli/cli_grammar.rs");
include!("open_cli/file_access.rs");
include!("open_cli/source_identity.rs");
include!("open_cli/cleanup_and_types.rs");
include!("open_cli/read_only_policy.rs");
include!("open_cli/snapshot_reuse.rs");
include!("open_cli/defaults_and_usage.rs");
include!("open_cli/parquet_validation.rs");
