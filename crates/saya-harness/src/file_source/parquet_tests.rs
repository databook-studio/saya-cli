//! Behavioural coverage for the C2a Parquet file staging primitive: the
//! private-copy read, the bounded decode in a locked staging connection,
//! nested-type refusals, cap seams, and cancel semantics — including every
//! refusal leaving no file behind. Fixtures are generated with DuckDB itself
//! on a throwaway connection; no binary fixture is committed.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use duckdb::Connection;
use sha2::{Digest, Sha256};

use super::{
    CsvStageOptions, InferredType, ParquetCaps, RESERVED_METADATA_TABLE, STAGE_TIMEOUT,
    STAGED_DB_FILE, SourceFormat, StageError, parquet_stage, stage_parquet, stage_source,
};
use crate::HarnessError;

fn temp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-parquetstage-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temporary root");
    root
}

/// Generates one Parquet fixture with DuckDB itself on a throwaway in-memory
/// connection — the file on disk is the only artifact left behind.
fn write_parquet_fixture(path: &Path, select: &str) -> Vec<u8> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fixture parent");
    }
    let conn = Connection::open_in_memory().expect("fixture connection");
    conn.execute_batch(&format!(
        "COPY ({select}) TO '{}' (FORMAT parquet)",
        path.display()
    ))
    .expect("fixture parquet");
    fs::read(path).expect("fixture bytes")
}

fn source_read(path: &Path) -> super::read::SourceRead {
    let bytes = fs::read(path).expect("fixture bytes");
    let file_name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
    super::read::SourceRead {
        sha256: {
            let digest = Sha256::digest(&bytes);
            digest.iter().map(|byte| format!("{byte:02x}")).collect()
        },
        bytes: bytes.clone(),
        file_name,
        stem,
        size: bytes.len() as u64,
    }
}

fn flat_fixture(root: &Path, name: &str) -> PathBuf {
    let path = root.join("in").join(name);
    write_parquet_fixture(
        &path,
        "SELECT 1 AS id, 'a' AS name, 12.5 AS amount, TRUE AS ok, \
         DATE '2024-01-02' AS on_date, TIMESTAMP '2024-01-02 03:04:05' AS at_time, \
         NULL AS hole \
         UNION ALL SELECT 2, 'b', 7.0, FALSE, DATE '2024-02-03', TIMESTAMP '2024-02-03 09:00:00', NULL \
         UNION ALL SELECT 3, 'c', 10.0, TRUE, DATE '2024-03-04', TIMESTAMP '2024-03-04 11:30:00', NULL",
    );
    path
}

fn assert_dest_empty(dest: &Path) {
    let entries: Vec<String> = fs::read_dir(dest)
        .expect("destination exists")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(entries.is_empty(), "destination not empty: {entries:?}");
}

#[path = "parquet_tests/staging.rs"]
mod staging;

#[path = "parquet_tests/decode_shapes.rs"]
mod decode_shapes;

#[path = "parquet_tests/refusals.rs"]
mod refusals;

#[path = "parquet_tests/identity.rs"]
mod identity;

#[path = "parquet_tests/decode_budget.rs"]
mod decode_budget;
