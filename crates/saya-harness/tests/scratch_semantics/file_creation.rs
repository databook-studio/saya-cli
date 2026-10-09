//! What a ReadWrite open does to the filesystem: it creates the missing
//! database file, refuses to invent missing parent directories, and stamps
//! the mode the engine itself chooses.

use duckdb::Connection;

use super::{TempDir, open, scratch_config};

#[test]
fn read_write_open_creates_a_missing_database_file() {
    let dir = TempDir::new("create-file");
    let path = dir.path.join("scratch.duckdb");
    assert!(!path.exists());
    let _conn = open(&path);
    assert!(path.exists(), "ReadWrite open must create the missing file");
}

#[test]
fn read_write_open_does_not_create_missing_parent_directories() {
    let dir = TempDir::new("missing-parents");
    let path = dir.path.join("missing-one/missing-two/scratch.duckdb");
    assert!(Connection::open_with_flags(&path, scratch_config()).is_err());
    assert!(
        !dir.path.join("missing-one").exists(),
        "the engine must not create parent directories"
    );
}

#[cfg(unix)]
/// **Reality contradicted the design.** DuckDB creates the database file with
/// mode 0644 — group and world readable. The run directory is 0700
/// (`run_dir.rs`), so the file is unreachable in practice today, but the
/// scratch file carries no protection of its own: anything that ever widens
/// the run directory exposes every staged row. Pinned as observed, not as
/// hoped; the caller must chmod after create if it wants defence in depth.
#[test]
#[cfg(unix)]
fn a_created_database_file_is_created_group_and_world_readable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("mode");
    let path = dir.path.join("scratch.duckdb");
    let _conn = open(&path);
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o644,
        "observed DuckDB's own creation mode; if this changes, revisit the ADR"
    );
}
