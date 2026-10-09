#[test]
fn file_session_cleanup_removes_only_owned_state() {
    let root = isolated_root("cleanup");
    let content = "id\n1\n2\n";
    let csv = root.join("sales.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());
    let out = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));

    let foreign = root.join("files").join("not-a-saya-source");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("keep.txt"), "foreign state\n").unwrap();

    let cleanup = open_command(&root)
        .args(["open", "--cleanup", "all", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(cleanup.status.code(), Some(0), "{}", stderr_of(&cleanup));
    let stdout = stdout_of(&cleanup);
    assert!(
        stdout.contains("Removed:"),
        "cleanup prints what it removed: {stdout}"
    );
    assert!(stdout.contains(&sha[..12]), "{stdout}");
    assert!(
        snapshot_dirs(&root, &sha).is_empty(),
        "the staged source is removed"
    );
    assert!(
        foreign.join("keep.txt").exists(),
        "a foreign directory under the root survives cleanup --all"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn typed_table_reports_cast_failures() {
    let root = isolated_root("typed-failures");
    let mut content = String::from("n,label\n");
    for value in 1..=1000 {
        content.push_str(&format!("{value},ok\n"));
    }
    content.push_str("x,bad\n");
    let csv = root.join("metrics.csv");
    fs::write(&csv, &content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let out = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--typed",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("n: 1 of 1001 values did not cast"),
        "the cast failure is reported, never silently: {stdout}"
    );
    assert!(
        stdout.contains("kept NULL"),
        "the failure line says what happened to the values: {stdout}"
    );

    let count = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_metrics",
        "SELECT count(*) FROM metrics_typed",
    )
    .output()
    .unwrap();
    assert_eq!(count.status.code(), Some(0), "{}", stderr_of(&count));
    assert!(
        stdout_of(&count).contains("1001"),
        "the typed table has every row: {}",
        stdout_of(&count)
    );
    let _ = fs::remove_dir_all(&root);
}
