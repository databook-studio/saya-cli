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

#[test]
fn stages_a_csv_into_a_private_duckdb_file_with_metadata_and_preview() {
    let root = temp_root("happy");
    let source = root.join("in").join("sales_2024.csv");
    write_bytes(&source, SALES_CSV);
    let dest = root.join("dest");
    let staged = stage_csv(
        &source,
        &dest,
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("staging succeeds");
    assert_eq!(staged.db_path, dest.join(STAGED_DB_FILE));
    assert_eq!(staged.table, "sales_2024");
    assert_eq!(
        staged.columns,
        [
            "order_id",
            "total",
            "active",
            "ordered_on",
            "ordered_at",
            "note"
        ]
    );
    assert_eq!(staged.rows, 3);
    assert_eq!(staged.bytes, SALES_CSV.len() as u64);
    assert_eq!(staged.sha256, hex(SALES_CSV));

    let preview = &staged.preview;
    assert_eq!(preview.delimiter, b',');
    assert!(preview.header);
    assert_eq!(
        infer_of(&staged),
        [
            InferredType::Integer,
            InferredType::Decimal,
            InferredType::Boolean,
            InferredType::Date,
            InferredType::Timestamp,
            InferredType::Text,
        ]
    );
    let nulls: Vec<usize> = preview
        .columns
        .iter()
        .map(|column| column.null_count)
        .collect();
    assert_eq!(nulls, [0, 0, 0, 0, 0, 1]);
    assert_eq!(preview.sample_rows.len(), 3);
    assert_eq!(preview.sample_rows[1][5], "");

    let connection = Connection::open(&staged.db_path).expect("staged file reopens");
    let rows: i64 = connection
        .query_row(
            "SELECT count(*) FROM sales_2024",
            duckdb::params![],
            |row| row.get(0),
        )
        .expect("data table queryable");
    assert_eq!(rows, 3);
    let mut types = connection
        .prepare(
            "SELECT data_type FROM information_schema.columns \
             WHERE table_name = 'sales_2024' ORDER BY ordinal_position",
        )
        .expect("column introspection");
    let types: Vec<String> = types
        .query_map(duckdb::params![], |row| row.get(0))
        .expect("column scan")
        .collect::<Result<_, _>>()
        .expect("column types");
    assert_eq!(types, ["VARCHAR"; 6]);

    let value_of = |key: &str| -> String {
        connection
            .query_row(
                "SELECT value FROM saya_file_source WHERE key = ?",
                duckdb::params![key],
                |row| row.get(0),
            )
            .expect("metadata row")
    };
    assert_eq!(value_of("format"), "csv");
    assert_eq!(value_of("rows"), "3");
    assert_eq!(value_of("columns"), "6");
    assert_eq!(value_of("file_name"), "sales_2024.csv");
    assert_eq!(value_of("sha256"), staged.sha256);
    assert_eq!(value_of("bytes"), SALES_CSV.len().to_string());
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
    assert!(
        !leftovers.iter().any(|name| name.ends_with(".wal")),
        "no wal left: {leftovers:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn infers_column_types_from_the_sampled_rows() {
    let root = temp_root("infer");
    let source = root.join("in").join("types.csv");
    write_bytes(
        &source,
        b"n,d,b,dt,ts,txt,mixed,holes\n\
          1,1.5,true,2024-01-02,2024-01-02T03:04:05Z,hello,1,\n\
          2,2.5,false,2024-02-03,2024-03-04T05:06:07+00:30,world,x,\n",
    );
    let dest = root.join("dest");
    let staged = stage_csv(
        &source,
        &dest,
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("staging succeeds");
    assert_eq!(
        infer_of(&staged),
        [
            InferredType::Integer,
            InferredType::Decimal,
            InferredType::Boolean,
            InferredType::Date,
            InferredType::Timestamp,
            InferredType::Text,
            InferredType::Text,
            InferredType::Text,
        ]
    );
    assert_eq!(staged.preview.columns[7].null_count, 2);
    assert_eq!(staged.preview.sample_rows.len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn leading_zero_numerics_and_non_iso_dates_stay_text() {
    let root = temp_root("leading");
    let source = root.join("in").join("leading.csv");
    write_bytes(&source, b"code,amount,when\n007,008.5,2024-1-5\n");
    let dest = root.join("dest");
    let staged = stage_csv(
        &source,
        &dest,
        CsvStageOptions {
            delimiter: Some(b','),
            header: true,
        },
    )
    .expect("staging succeeds");
    assert_eq!(infer_of(&staged), [InferredType::Text; 3]);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn symlinked_source_is_refused() {
    let root = temp_root("symlink");
    let real = root.join("in").join("data.csv");
    write_bytes(&real, b"a,b\n1,2\n");
    let link = root.join("in").join("link.csv");
    std::os::unix::fs::symlink(&real, &link).expect("symlink fixture");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let error = stage_csv(
        &link,
        &dest,
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect_err("symlink refused");
    assert!(matches!(
        error,
        StageError::Source(HarnessError::SymlinkRefused { .. })
    ));
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn directory_source_is_refused() {
    let root = temp_root("directory");
    let source = root.join("in").join("data.csv");
    fs::create_dir_all(&source).expect("directory fixture");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let error = stage_csv(
        &source,
        &dest,
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect_err("directory refused");
    assert!(matches!(
        error,
        StageError::Source(HarnessError::NotRegularFile { .. })
    ));
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn oversize_source_is_refused() {
    let root = temp_root("oversize");
    let source = root.join("in").join("big.csv");
    write_bytes(&source, &vec![b'x'; SOURCE_CAP + 1]);
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let error = stage_csv(
        &source,
        &dest,
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect_err("oversize refused");
    let StageError::Source(HarnessError::BoundsExceeded { max, .. }) = error else {
        panic!("expected a bounds refusal, got {error:?}");
    };
    assert_eq!(max, SOURCE_CAP as u64);
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ragged_rows_are_refused_and_leave_no_file() {
    let root = temp_root("ragged");
    let source = root.join("in").join("ragged.csv");
    write_bytes(&source, b"a,b\n1,2\n3,4,5\n");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let error = stage_csv(
        &source,
        &dest,
        CsvStageOptions {
            delimiter: Some(b','),
            header: true,
        },
    )
    .expect_err("ragged refused");
    assert_eq!(error.to_string(), "CSV data row 2 has 3 fields; expected 2");
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn same_bytes_stage_to_the_same_sha256() {
    let root = temp_root("sha");
    let bytes = b"a,b\n1,2\n";
    write_bytes(&root.join("in").join("one.csv"), bytes);
    write_bytes(&root.join("in").join("two.csv"), bytes);
    let staged_one = stage_csv(
        &root.join("in").join("one.csv"),
        &root.join("d1"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("first staging");
    let staged_two = stage_csv(
        &root.join("in").join("two.csv"),
        &root.join("d2"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("second staging");
    let restaged = stage_csv(
        &root.join("in").join("one.csv"),
        &root.join("d3"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("third staging");
    assert_eq!(staged_one.sha256, staged_two.sha256);
    assert_eq!(staged_one.sha256, restaged.sha256);
    assert_eq!(staged_one.sha256.len(), 64);
    assert!(
        staged_one
            .sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn staging_never_opens_the_parents_other_files() {
    let root = temp_root("sibling");
    let dir = root.join("in");
    write_bytes(&dir.join("data.csv"), b"a,b\n1,2\n");
    let locked = dir.join("locked.csv");
    fs::write(&locked, b"unreadable sibling").expect("sibling fixture");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("lock sibling");
    let staged = stage_csv(
        &dir.join("data.csv"),
        &root.join("dest"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    );
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).expect("unlock sibling");
    assert!(
        staged.is_ok(),
        "staging succeeds beside an unreadable sibling"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn elapsed_deadline_rolls_back_and_leaves_no_file() {
    let root = temp_root("deadline");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let source = read::SourceRead {
        bytes: b"a,b\n1,2\n".to_vec(),
        sha256: "0".repeat(64),
        file_name: "data.csv".to_owned(),
        stem: "data".to_owned(),
        size: 8,
    };
    let error = csv_stage::write_staged(
        &dest,
        "data",
        &["a".to_owned(), "b".to_owned()],
        &[vec!["1".to_owned(), "2".to_owned()]],
        &source,
        Instant::now(),
    )
    .expect_err("elapsed deadline refuses");
    assert!(matches!(error, StageError::Timeout));
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn failed_database_write_leaves_no_file() {
    let root = temp_root("dbfail");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let source = read::SourceRead {
        bytes: b"a,b\n1,2\n".to_vec(),
        sha256: "0".repeat(64),
        file_name: "data.csv".to_owned(),
        stem: "data".to_owned(),
        size: 8,
    };
    // An empty column list produces invalid `CREATE TABLE` SQL, so the write
    // fails at the database layer after the temp file already exists — the
    // cleanup guard must leave nothing behind.
    let error = csv_stage::write_staged(
        &dest,
        "data",
        &[],
        &[],
        &source,
        Instant::now() + STAGE_TIMEOUT,
    )
    .expect_err("empty column list refused");
    assert!(matches!(error, StageError::Database));
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn empty_csv_is_refused() {
    let root = temp_root("empty");
    let source = root.join("in").join("empty.csv");
    write_bytes(&source, b"");
    let error = stage_csv(
        &source,
        &root.join("dest"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect_err("empty file refused");
    assert!(matches!(error, StageError::Empty));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn header_only_csv_stages_an_empty_table() {
    let root = temp_root("headeronly");
    let source = root.join("in").join("header.csv");
    write_bytes(&source, b"a,b\n");
    let staged = stage_csv(
        &source,
        &root.join("dest"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("header-only staging succeeds");
    assert_eq!(staged.rows, 0);
    assert_eq!(staged.columns, ["a", "b"]);
    assert!(staged.preview.sample_rows.is_empty());
    assert_eq!(staged.preview.columns[0].null_count, 0);
    let connection = Connection::open(&staged.db_path).expect("staged file reopens");
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM header", duckdb::params![], |row| {
            row.get(0)
        })
        .expect("empty table queryable");
    assert_eq!(rows, 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reserved_metadata_table_name_is_refused() {
    let root = temp_root("reserved");
    let source = root.join("in").join("saya_file_source.csv");
    write_bytes(&source, b"a,b\n1,2\n");
    let dest = root.join("dest");
    let error = stage_csv(
        &source,
        &dest,
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect_err("reserved name refused");
    assert!(matches!(
        error,
        StageError::ReservedTableName { name } if name == RESERVED_METADATA_TABLE
    ));
    assert!(!dest.join(STAGED_DB_FILE).exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unspecified_delimiter_is_sniffed_and_reported() {
    let root = temp_root("sniff");
    write_bytes(&root.join("in").join("semi.csv"), b"a;b\n1;2\n");
    write_bytes(&root.join("in").join("comma.csv"), b"a,b\n1,2\n");
    let staged = stage_csv(
        &root.join("in").join("semi.csv"),
        &root.join("d1"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("semicolon staging");
    assert_eq!(staged.preview.delimiter, b';');
    assert_eq!(staged.columns, ["a", "b"]);
    let staged = stage_csv(
        &root.join("in").join("comma.csv"),
        &root.join("d2"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("comma staging");
    assert_eq!(staged.preview.delimiter, b',');
    let _ = fs::remove_dir_all(root);
}

#[test]
fn headerless_csv_uses_generated_column_names() {
    let root = temp_root("headerless");
    let source = root.join("in").join("data.csv");
    write_bytes(&source, b"Ada;42\nGrace;99\n");
    let staged = stage_csv(
        &source,
        &root.join("dest"),
        CsvStageOptions {
            delimiter: Some(b';'),
            header: false,
        },
    )
    .expect("headerless staging");
    assert_eq!(staged.columns, ["column_1", "column_2"]);
    assert!(!staged.preview.header);
    assert_eq!(staged.preview.sample_rows[0], ["Ada", "42"]);
    assert_eq!(staged.rows, 2);
    let connection = Connection::open(&staged.db_path).expect("staged file reopens");
    let value: String = connection
        .query_row(
            "SELECT column_2 FROM data WHERE column_1 = 'Grace'",
            duckdb::params![],
            |row| row.get(0),
        )
        .expect("headerless row");
    assert_eq!(value, "99");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn invalid_explicit_delimiter_is_refused() {
    let root = temp_root("delimiter");
    let source = root.join("in").join("data.csv");
    write_bytes(&source, b"a,b\n1,2\n");
    let error = stage_csv(
        &source,
        &root.join("dest"),
        CsvStageOptions {
            delimiter: Some(b'"'),
            header: true,
        },
    )
    .expect_err("quote-as-delimiter refused");
    assert!(matches!(
        error,
        StageError::Csv(crate::scratch::CsvError::InvalidDelimiter)
    ));
    let _ = fs::remove_dir_all(root);
}
