/// C2a: a local Parquet file stages through the bounded private decode into
/// the same snapshot layout, previews with `Format: parquet` and native
/// column types, queries through the generated read-only profile, and the
/// session's read-only gate refuses `read_parquet` (the staged snapshot must
/// never be able to open other files).
#[test]
fn open_stages_a_parquet_file_and_refuses_file_functions() {
    let root = isolated_root("parquet");
    let pq = root.join("sales_2024.parquet");
    {
        let conn = duckdb::Connection::open_in_memory().expect("fixture connection");
        conn.execute_batch(&format!(
            "COPY (SELECT 1 AS id, 'widget' AS name, 9.5 AS price \
             UNION ALL SELECT 2, 'gadget', 19.99 \
             UNION ALL SELECT 3, 'sprocket', NULL) TO '{}' (FORMAT parquet)",
            pq.display()
        ))
        .expect("fixture parquet");
    }
    let bytes = fs::read(&pq).unwrap();
    let sha = sha256_hex(&bytes);

    let out = open_command(&root)
        .args(["open", pq.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "open must succeed headless, stderr: {}",
        stderr_of(&out)
    );
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("File: sales_2024.parquet"),
        "names the file: {stdout}"
    );
    assert!(
        stdout.contains("Format: parquet"),
        "reports the parquet format: {stdout}"
    );
    assert!(stdout.contains("Rows: 3"), "shows the row count: {stdout}");
    assert!(
        stdout.contains("id: integer"),
        "shows the native type: {stdout}"
    );
    assert!(
        stdout.contains("price: decimal (1 nulls)"),
        "shows null counts: {stdout}"
    );
    assert!(
        !stdout.contains("Stored as text columns"),
        "parquet keeps native types, the CSV text-storage line does not apply: {stdout}"
    );
    assert!(
        stdout.contains("profile file_sales_2024"),
        "names the profile: {stdout}"
    );

    let dir = snapshot_dir(&root, &sha);
    let db = dir.join("source.duckdb");
    assert!(
        db.exists(),
        "the staged db lands at <root>/<sha16>-<digest>/source.duckdb"
    );
    let mode = fs::metadata(&db).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "the staged db file is 0600");
    let dir_mode = fs::metadata(&dir).unwrap().permissions().mode();
    assert_eq!(dir_mode & 0o777, 0o700, "the snapshot directory is 0700");
    assert!(
        dir.join("connections.toml").exists(),
        "connections.toml sits beside the db"
    );

    let select = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales_2024",
        "SELECT count(*) FROM sales_2024",
    )
    .output()
    .unwrap();
    assert_eq!(
        select.status.code(),
        Some(0),
        "select must succeed through the generated profile, stderr: {}",
        stderr_of(&select)
    );

    let refused = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales_2024",
        "SELECT * FROM read_parquet('/etc/passwd')",
    )
    .output()
    .unwrap();
    assert_eq!(
        refused.status.code(),
        Some(4),
        "read_parquet must be refused by the session's safety gate: {} {}",
        stdout_of(&refused),
        stderr_of(&refused)
    );
    assert!(
        (stdout_of(&refused) + &stderr_of(&refused)).contains("read_parquet"),
        "the refusal names the function"
    );
    let _ = fs::remove_dir_all(&root);
}

/// CSV-only flags on a Parquet file are a usage error, refused before (by
/// extension) or after (by PAR1 magic) staging — never silently ignored.
#[test]
fn open_parquet_refuses_csv_only_flags() {
    let root = isolated_root("parquet-flags");
    let pq = root.join("measurements.parquet");
    {
        let conn = duckdb::Connection::open_in_memory().expect("fixture connection");
        conn.execute_batch(&format!(
            "COPY (SELECT 1 AS reading) TO '{}' (FORMAT parquet)",
            pq.display()
        ))
        .expect("fixture parquet");
    }
    for flags in [
        vec!["--typed"],
        vec!["--delimiter", ";"],
        vec!["--no-header"],
    ] {
        let out = open_command(&root)
            .arg("open")
            .arg(pq.to_str().unwrap())
            .arg("--non-interactive")
            .args(&flags)
            .output()
            .unwrap();
        assert_ne!(
            out.status.code(),
            Some(0),
            "CSV-only flags {flags:?} must be refused on a Parquet file: {} {}",
            stdout_of(&out),
            stderr_of(&out)
        );
        assert!(
            (stdout_of(&out) + &stderr_of(&out)).contains("Parquet"),
            "the refusal names the format conflict: {} {}",
            stdout_of(&out),
            stderr_of(&out)
        );
    }
    // The snapshot still stages and reopens without the flags.
    let bytes = fs::read(&pq).unwrap();
    let sha = sha256_hex(&bytes);
    let out = open_command(&root)
        .args(["open", pq.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(
        snapshot_dirs(&root, &sha)
            .first()
            .is_some_and(|dir| dir.join("source.duckdb").exists()),
        "the snapshot is staged"
    );
    let _ = fs::remove_dir_all(&root);
}
