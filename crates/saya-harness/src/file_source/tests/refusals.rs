use super::*;

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
