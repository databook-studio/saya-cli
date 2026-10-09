use super::*;

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
