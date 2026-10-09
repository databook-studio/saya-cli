#[test]
fn file_session_needs_no_database_profile() {
    let root = isolated_root("main");
    let csv = root.join("sales.csv");
    let content = "id,name,price\n1,widget,9.5\n2,gadget,19.99\n3,sprocket,\n";
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let out = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
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
        stdout.contains("File: sales.csv"),
        "names the file: {stdout}"
    );
    assert!(
        stdout.contains(&format!("SHA-256: {}", &sha[..12])),
        "shows the sha prefix: {stdout}"
    );
    assert!(stdout.contains("Rows: 3"), "shows the row count: {stdout}");
    assert!(
        stdout.contains("id: integer"),
        "shows inferred types: {stdout}"
    );
    assert!(
        stdout.contains("price: decimal (1 nulls)"),
        "shows null counts: {stdout}"
    );
    assert!(
        stdout.contains("Stored as text columns; use --typed for a typed copy."),
        "states the text storage: {stdout}"
    );
    assert!(
        stdout.contains("profile file_sales"),
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
    let connections = dir.join("connections.toml");
    assert!(connections.exists(), "connections.toml sits beside the db");

    let select = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales",
        "SELECT count(*) FROM sales",
    )
    .output()
    .unwrap();
    assert_eq!(
        select.status.code(),
        Some(0),
        "select must succeed through the generated profile, stderr: {}",
        stderr_of(&select)
    );
    assert!(
        stdout_of(&select).contains("\"event\":\"query_result\""),
        "select renders a query result: {}",
        stdout_of(&select)
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn open_one_file_does_not_grant_parent_access() {
    let root = isolated_root("parent");
    let content = "id,when\n1,2026-01-02\n2,2026-02-03\n";
    fs::write(root.join("sales.csv"), content).unwrap();
    fs::write(root.join("secret.txt"), "secret-value\n").unwrap();
    let sha = sha256_hex(content.as_bytes());

    let out = open_command(&root)
        .args([
            "open",
            root.join("sales.csv").to_str().unwrap(),
            "--non-interactive",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));

    let tables = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales",
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema NOT IN ('information_schema', 'pg_catalog') ORDER BY table_name",
    )
    .output()
    .unwrap();
    assert_eq!(tables.status.code(), Some(0), "{}", stderr_of(&tables));
    let stdout = stdout_of(&tables);
    assert!(
        stdout.contains("sales"),
        "the staged table is there: {stdout}"
    );
    assert!(
        stdout.contains("saya_file_source"),
        "the metadata table is there: {stdout}"
    );
    assert!(
        !stdout.contains("secret"),
        "nothing from the parent directory is reachable: {stdout}"
    );

    let probe = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales",
        &format!(
            "SELECT * FROM read_csv('{}')",
            root.join("secret.txt").display()
        ),
    )
    .output()
    .unwrap();
    assert_eq!(
        probe.status.code(),
        Some(4),
        "read_csv must be refused by the safety gate: {} {}",
        stdout_of(&probe),
        stderr_of(&probe)
    );
    let _ = fs::remove_dir_all(&root);
}
