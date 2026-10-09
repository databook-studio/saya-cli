use super::*;

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
