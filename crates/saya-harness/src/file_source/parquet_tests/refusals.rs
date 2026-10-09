use super::*;

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
