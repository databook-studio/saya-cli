use super::*;

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
