use super::*;

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
