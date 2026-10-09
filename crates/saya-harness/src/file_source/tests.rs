//! Behavioural coverage for the C1a CSV file staging primitive: the contained
//! single-file read, the transactional DuckDB staging writer, type inference,
//! and the preview — including every refusal leaving no file behind.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Instant,
};

use duckdb::Connection;
use sha2::{Digest, Sha256};

use super::{
    CsvStageOptions, InferredType, RESERVED_METADATA_TABLE, STAGE_TIMEOUT, STAGED_DB_FILE,
    StageError, StagedSource, csv_stage, read, stage_csv,
};
use crate::HarnessError;
use crate::workspace::MAX_SCRATCH_IMPORT_BYTES as SOURCE_CAP;

const SALES_CSV: &[u8] = b"order_id,total,active,ordered_on,ordered_at,note\n\
1,12.5,true,2024-01-02,2024-01-02T03:04:05Z,ok\n\
2,7.0,false,2024-02-03,2024-02-03T09:00:00+02:00,\n\
3,10,true,2024-03-04,2024-03-04T11:30:00Z,pending\n";

fn temp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-filestage-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temporary root");
    root
}

fn write_bytes(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fixture parent");
    }
    fs::write(path, bytes).expect("fixture file");
}

fn hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn assert_dest_empty(dest: &Path) {
    let entries: Vec<String> = fs::read_dir(dest)
        .expect("destination exists")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(entries.is_empty(), "destination not empty: {entries:?}");
}

fn infer_of(staged: &StagedSource) -> Vec<InferredType> {
    staged
        .preview
        .columns
        .iter()
        .map(|column| column.inferred)
        .collect()
}

#[path = "tests/staging.rs"]
mod staging;

#[path = "tests/inference.rs"]
mod inference;

#[path = "tests/refusals.rs"]
mod refusals;

#[path = "tests/transactional.rs"]
mod transactional;
