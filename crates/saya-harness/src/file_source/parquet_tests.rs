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
    CsvStageOptions, ParquetCaps, RESERVED_METADATA_TABLE, STAGED_DB_FILE, SourceFormat,
    StageError, parquet_stage, stage_parquet, stage_source,
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

#[test]
fn stages_a_parquet_into_a_private_duckdb_file_with_metadata_and_preview() {
    let root = temp_root("happy");
    let source = flat_fixture(&root, "sales_2024.parquet");
    let bytes = fs::read(&source).expect("fixture bytes");
    let dest = root.join("dest");
    let staged = stage_parquet(&source, &dest).expect("staging succeeds");
    assert_eq!(staged.db_path, dest.join(STAGED_DB_FILE));
    assert_eq!(staged.table, "sales_2024");
    assert_eq!(staged.format, SourceFormat::Parquet);
    assert_eq!(
        staged.columns,
        ["id", "name", "amount", "ok", "on_date", "at_time", "hole"]
    );
    assert_eq!(staged.rows, 3);
    assert_eq!(staged.bytes, bytes.len() as u64);
    assert_eq!(staged.sha256.len(), 64);

    let preview = &staged.preview;
    let inferred: Vec<_> = preview.columns.iter().map(|c| c.inferred).collect();
    assert_eq!(
        inferred,
        [
            super::InferredType::Integer,
            super::InferredType::Text,
            super::InferredType::Decimal,
            super::InferredType::Boolean,
            super::InferredType::Date,
            super::InferredType::Timestamp,
            super::InferredType::Integer,
        ]
    );
    let nulls: Vec<usize> = preview.columns.iter().map(|c| c.null_count).collect();
    assert_eq!(nulls, [0, 0, 0, 0, 0, 0, 3]);
    assert_eq!(preview.sample_rows.len(), 3);
    assert_eq!(
        preview.sample_rows[0],
        [
            "1",
            "a",
            "12.5",
            "true",
            "2024-01-02",
            "2024-01-02 03:04:05",
            ""
        ]
    );

    let connection = Connection::open(&staged.db_path).expect("staged file reopens");
    let rows: i64 = connection
        .query_row(
            "SELECT count(*) FROM sales_2024",
            duckdb::params![],
            |row| row.get(0),
        )
        .expect("data table queryable");
    assert_eq!(rows, 3);
    let value_of = |key: &str| -> String {
        connection
            .query_row(
                "SELECT value FROM saya_file_source WHERE key = ?",
                duckdb::params![key],
                |row| row.get(0),
            )
            .expect("metadata row")
    };
    assert_eq!(value_of("format"), "parquet");
    assert_eq!(value_of("rows"), "3");
    assert_eq!(value_of("columns"), "7");
    assert_eq!(value_of("file_name"), "sales_2024.parquet");
    assert_eq!(value_of("sha256"), staged.sha256);
    assert!(
        value_of("staged_unix_ms")
            .parse::<u128>()
            .is_ok_and(|ms| ms > 0)
    );
    drop(connection);

    #[cfg(unix)]
    {
        let file_mode = fs::metadata(&staged.db_path).unwrap().permissions().mode();
        assert_eq!(file_mode & 0o777, 0o600);
        let dir_mode = fs::metadata(&dest).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700);
    }
    let leftovers: Vec<String> = fs::read_dir(&dest)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        leftovers,
        [STAGED_DB_FILE.to_owned()],
        "only the staged db remains — the private copy is deleted: {leftovers:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parquet_decode_cap_prevents_expansion() {
    let root = temp_root("cap");
    let source = flat_fixture(&root, "small.parquet");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let read = source_read(&source);
    let caps = ParquetCaps {
        max_rows: 2,
        ..ParquetCaps::default()
    };
    let error = parquet_stage::stage_read(read, &dest, caps).expect_err("cap refusal");
    assert!(
        matches!(error, StageError::ParquetTooManyRows { rows: 3, max: 2 }),
        "typed row-cap refusal, got {error:?}"
    );
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parquet_column_cap_is_enforced() {
    let root = temp_root("cols");
    let source = flat_fixture(&root, "three.parquet");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let read = source_read(&source);
    let caps = ParquetCaps {
        max_columns: 2,
        ..ParquetCaps::default()
    };
    let error = parquet_stage::stage_read(read, &dest, caps).expect_err("column cap refusal");
    assert!(
        matches!(
            error,
            StageError::ParquetTooManyColumns { columns: 3, max: 2 }
        ),
        "typed column-cap refusal, got {error:?}"
    );
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parquet_nested_types_fail_explicitly() {
    let root = temp_root("nested");
    let source = root.join("in").join("nested.parquet");
    write_parquet_fixture(&source, "SELECT {'a': 1, 'b': 'x'} AS s, [1, 2, 3] AS l");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let error = stage_parquet(&source, &dest).expect_err("nested refused");
    let StageError::ParquetNestedColumn { column, kind } = error else {
        panic!("expected a nested-column refusal, got {error:?}");
    };
    assert!(column == "s" || column == "l", "names the column: {column}");
    let upper = kind.to_ascii_uppercase();
    assert!(
        upper.contains("STRUCT(") || upper.contains('['),
        "shows the nested type: {kind}"
    );
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parquet_empty_file_stages_an_empty_table() {
    let root = temp_root("empty");
    let source = root.join("in").join("empty.parquet");
    write_parquet_fixture(&source, "SELECT 1 AS id, 'x' AS name WHERE false");
    let dest = root.join("dest");
    let staged = stage_parquet(&source, &dest).expect("empty parquet stages");
    assert_eq!(staged.rows, 0);
    assert_eq!(staged.columns, ["id", "name"]);
    assert!(staged.preview.sample_rows.is_empty());
    let connection = Connection::open(&staged.db_path).expect("staged file reopens");
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM empty", duckdb::params![], |row| {
            row.get(0)
        })
        .expect("empty table queryable");
    assert_eq!(rows, 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parquet_reserved_metadata_table_name_is_refused() {
    let root = temp_root("reserved");
    let source = root.join("in").join("saya_file_source.parquet");
    write_parquet_fixture(&source, "SELECT 1 AS a");
    let dest = root.join("dest");
    let error = stage_parquet(&source, &dest).expect_err("reserved name refused");
    assert!(matches!(
        error,
        StageError::ReservedTableName { name } if name == RESERVED_METADATA_TABLE
    ));
    assert!(!dest.join(STAGED_DB_FILE).exists());
    let _ = fs::remove_dir_all(root);
}

/// The cap seam doubles as the wall-clock seam: a tiny timeout with a fixture
/// whose decode outruns it means the watchdog interrupts a statement in
/// flight. Any cancel path must leave the destination empty.
#[test]
fn parquet_cancel_leaves_no_partial_table() {
    let root = temp_root("cancel");
    let source = root.join("in").join("wide.parquet");
    write_parquet_fixture(
        &source,
        "SELECT i, i::VARCHAR AS s, i % 7 AS m, i * 3.1 AS d FROM range(400000) t(i)",
    );
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let read = source_read(&source);
    let caps = ParquetCaps {
        timeout: Duration::from_millis(300),
        ..ParquetCaps::default()
    };
    let error = parquet_stage::stage_read(read, &dest, caps).expect_err("timeout refuses");
    assert!(
        matches!(error, StageError::Timeout),
        "expected the timeout refusal, got {error:?}"
    );
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parquet_elapsed_deadline_refuses_before_any_statement() {
    let root = temp_root("elapsed");
    let source = flat_fixture(&root, "flat.parquet");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let read = source_read(&source);
    let caps = ParquetCaps {
        timeout: Duration::ZERO,
        ..ParquetCaps::default()
    };
    let error = parquet_stage::stage_read(read, &dest, caps).expect_err("elapsed deadline");
    assert!(matches!(error, StageError::Timeout));
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stage_source_routes_by_extension_and_magic() {
    let root = temp_root("route");
    // Extension wins for a real Parquet file…
    let parquet = flat_fixture(&root, "data.parquet");
    let staged = stage_source(
        &parquet,
        &root.join("d1"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("parquet by extension");
    assert_eq!(staged.format, SourceFormat::Parquet);
    assert_eq!(staged.table, "data");
    // …and for a misnamed file whose bytes are not Parquet: an honest
    // refusal, never a silent CSV interpretation.
    let misnamed = root.join("in").join("not-parquet.parquet");
    fs::write(&misnamed, b"a,b\n1,2\n").expect("csv bytes");
    let error = stage_source(
        &misnamed,
        &root.join("d2"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect_err("csv bytes in a .parquet file refused");
    assert!(
        matches!(error, StageError::ParquetFailed(_) | StageError::Source(_)),
        "an honest refusal, got {error:?}"
    );
    // PAR1 magic routes an extensionless file to the Parquet path.
    let magic = root.join("in").join("extensionless.dat");
    fs::copy(&parquet, &magic).expect("parquet bytes");
    let staged = stage_source(
        &magic,
        &root.join("d3"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("parquet by magic");
    assert_eq!(staged.format, SourceFormat::Parquet);
    assert_eq!(staged.table, "extensionless");
    // CSV bytes without a .parquet name stay on the CSV path.
    let csv = root.join("in").join("plain.dat");
    fs::write(&csv, b"a,b\n1,2\n").expect("csv bytes");
    let staged = stage_source(
        &csv,
        &root.join("d4"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("csv by default");
    assert_eq!(staged.format, SourceFormat::Csv);
    assert_eq!(staged.columns, ["a", "b"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parquet_source_refusals_match_the_csv_ones() {
    let root = temp_root("refusals");
    // Symlink, non-regular file, and oversize refusals come from the same
    // contained single-file read the CSV path uses.
    let real = root.join("in").join("data.parquet");
    write_parquet_fixture(&real, "SELECT 1 AS a");
    let link = root.join("in").join("link.parquet");
    std::os::unix::fs::symlink(&real, &link).expect("symlink fixture");
    let error = stage_parquet(&link, &root.join("dest")).expect_err("symlink refused");
    assert!(matches!(
        error,
        StageError::Source(HarnessError::SymlinkRefused { .. })
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn same_parquet_bytes_stage_to_the_same_sha256() {
    let root = temp_root("sha");
    let one = flat_fixture(&root, "one.parquet");
    let two = root.join("in").join("two.parquet");
    fs::copy(&one, &two).expect("same bytes");
    let staged_one = stage_parquet(&one, &root.join("d1")).expect("first staging");
    let staged_two = stage_parquet(&two, &root.join("d2")).expect("second staging");
    assert_eq!(staged_one.sha256, staged_two.sha256);
    assert_eq!(staged_one.sha256.len(), 64);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn parquet_stage_respects_the_stage_timeout_constant() {
    let caps = ParquetCaps::default();
    assert_eq!(caps.max_rows, 500_000);
    assert_eq!(caps.max_columns, 512);
    assert_eq!(caps.max_decoded_bytes, 64 * 1024 * 1024);
    assert_eq!(caps.timeout, super::STAGE_TIMEOUT);
}

/// The dictionary bomb at the row cap: 500,000 rows repeating one 1,024-byte
/// value — a ~6 KiB file — decodes to 512,000,000 accounted bytes, far over
/// the 64 MiB budget. The refusal must happen by a streaming aggregate over
/// the source, BEFORE the decode's CREATE TABLE runs: the create probe stays
/// at zero and no staging artifact is left behind.
#[test]
fn parquet_row_cap_dictionary_bomb_refuses_before_any_create() {
    let root = temp_root("budget-bomb");
    let source = root.join("in").join("bomb.parquet");
    write_parquet_fixture(
        &source,
        "SELECT repeat('x', 1024) AS payload FROM range(500000)",
    );
    let on_disk = fs::read(&source).expect("fixture bytes").len();
    assert!(
        on_disk < 1_048_576,
        "the dictionary bomb must be tiny on disk: {on_disk} bytes"
    );
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let create_count = AtomicUsize::new(0);
    let error = parquet_stage::stage_inner(
        source_read(&source),
        &dest,
        ParquetCaps::default(),
        Some(&create_count),
    )
    .expect_err("the bomb is refused");
    assert!(
        matches!(
            error,
            StageError::ParquetTooManyDecodedBytes {
                bytes: 512_000_000,
                max: 67_108_864
            }
        ),
        "typed budget refusal naming the limit, got {error:?}"
    );
    assert_eq!(
        create_count.load(Ordering::Relaxed),
        0,
        "the decode's CREATE TABLE must never run"
    );
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

/// The audit regression (A922-6): a tiny, highly compressed Parquet file —
/// 80 rows of 1,048,576-byte text values, far under the row and column caps —
/// decoded to 83,886,080 bytes of cell data and staged. The 64 MiB accounted
/// decoded-byte budget must refuse the whole staging, naming the limit, and
/// leave no partial snapshot.
#[test]
fn parquet_decoded_byte_budget_refuses_high_compression() {
    let root = temp_root("budget-over");
    let source = root.join("in").join("tiny.parquet");
    write_parquet_fixture(
        &source,
        "SELECT repeat('x', 1048576) AS payload FROM range(80)",
    );
    let on_disk = fs::read(&source).expect("fixture bytes").len();
    assert!(
        on_disk < 1_048_576,
        "the fixture must be high-compression on disk: {on_disk} bytes"
    );
    let dest = root.join("dest");
    let error = stage_parquet(&source, &dest).expect_err("the decoded-byte budget refuses");
    assert!(
        matches!(
            error,
            StageError::ParquetTooManyDecodedBytes {
                bytes: 83_886_080,
                max: 67_108_864
            }
        ),
        "typed decoded-byte refusal naming the limit, got {error:?}"
    );
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

/// 63 rows of the same 1,048,576-byte values decode to 66,060,288 accounted
/// bytes — just under the 64 MiB budget — and stage with the default caps.
#[test]
fn parquet_just_under_the_decoded_byte_budget_stages() {
    let root = temp_root("budget-under");
    let source = root.join("in").join("under.parquet");
    write_parquet_fixture(
        &source,
        "SELECT repeat('x', 1048576) AS payload FROM range(63)",
    );
    let dest = root.join("dest");
    let staged = stage_parquet(&source, &dest).expect("just under the budget stages");
    assert_eq!(staged.rows, 63);
    let connection = Connection::open(&staged.db_path).expect("staged file reopens");
    let decoded: i64 = connection
        .query_row(
            "SELECT sum(octet_length(CAST(payload AS BLOB))) FROM under",
            duckdb::params![],
            |row| row.get(0),
        )
        .expect("decoded sum");
    assert_eq!(decoded, 66_060_288);
    let _ = fs::remove_dir_all(root);
}

/// The accounting is the sum of decoded cell bytes: text and blob by octet
/// length, every other type at its fixed physical width. The flat fixture
/// decodes to exactly 60 bytes (3 × (4 INTEGER, 1 VARCHAR, 2 DECIMAL(3,1),
/// 1 BOOLEAN, 4 DATE, 8 TIMESTAMP; the all-NULL VARCHAR nothing)) and the
/// decimal/blob fixture to exactly 24 (3 × (2 DECIMAL(4,1), 2 BLOB,
/// 4 INTEGER)); each stages at its exact budget and refuses one byte under.
#[test]
fn parquet_decoded_byte_accounting_sums_cell_widths() {
    let root = temp_root("budget-widths");
    let flat = flat_fixture(&root, "widths.parquet");
    let exact = ParquetCaps {
        max_decoded_bytes: 60,
        ..ParquetCaps::default()
    };
    let staged = parquet_stage::stage_read(source_read(&flat), &root.join("d1"), exact)
        .expect("exactly the budget stages");
    assert_eq!(staged.rows, 3);
    let one_under = ParquetCaps {
        max_decoded_bytes: 59,
        ..ParquetCaps::default()
    };
    let dest = root.join("d2");
    fs::create_dir_all(&dest).expect("destination");
    let error = parquet_stage::stage_read(source_read(&flat), &dest, one_under)
        .expect_err("one accounted byte over refuses");
    assert!(
        matches!(
            error,
            StageError::ParquetTooManyDecodedBytes { bytes: 60, max: 59 }
        ),
        "got {error:?}"
    );
    assert_dest_empty(&dest);

    let mixed = root.join("in").join("mixed.parquet");
    write_parquet_fixture(
        &mixed,
        "SELECT 1::DECIMAL(4,1) AS d, 'ab'::BLOB AS b, 5 AS i \
         UNION ALL SELECT 2::DECIMAL(4,1), 'cd'::BLOB, 6 \
         UNION ALL SELECT 3::DECIMAL(4,1), 'ef'::BLOB, 7",
    );
    let exact = ParquetCaps {
        max_decoded_bytes: 24,
        ..ParquetCaps::default()
    };
    let staged = parquet_stage::stage_read(source_read(&mixed), &root.join("d3"), exact)
        .expect("decimal, blob, and integer widths accounted exactly");
    assert_eq!(staged.rows, 3);
    let one_under = ParquetCaps {
        max_decoded_bytes: 23,
        ..ParquetCaps::default()
    };
    let dest = root.join("d4");
    fs::create_dir_all(&dest).expect("destination");
    let error = parquet_stage::stage_read(source_read(&mixed), &dest, one_under)
        .expect_err("refused one accounted byte under");
    assert!(
        matches!(
            error,
            StageError::ParquetTooManyDecodedBytes { bytes: 24, max: 23 }
        ),
        "got {error:?}"
    );
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

/// Observable proxy for invariant 1: the staging connection is pointed at a
/// saya-owned copy inside the destination, never at the user's original path.
/// The original file is deleted after the contained read — staging must still
/// succeed, because DuckDB only ever sees the copy.
#[test]
fn staging_reads_only_the_saya_owned_copy() {
    let root = temp_root("owned-copy");
    let source = flat_fixture(&root, "owned.parquet");
    let read = source_read(&source);
    fs::remove_file(&source).expect("the original is gone after the contained read");
    let dest = root.join("dest");
    let staged = parquet_stage::stage_read(read, &dest, ParquetCaps::default())
        .expect("staging succeeds without the original file present");
    assert_eq!(staged.rows, 3);
    assert_eq!(staged.table, "owned");
    let _ = fs::remove_dir_all(root);
}
