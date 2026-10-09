#[test]
fn write_sql_on_the_file_profile_is_refused() {
    let root = isolated_root("refusal");
    let content = "id\n1\n2\n";
    let csv = root.join("sales.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());
    let out = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));

    let delete = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales",
        "DELETE FROM sales",
    )
    .output()
    .unwrap();
    assert_eq!(
        delete.status.code(),
        Some(4),
        "DELETE must be refused, stderr: {}",
        stderr_of(&delete)
    );
    assert!(
        stderr_of(&delete).contains("\"event\":\"error\""),
        "refusal renders an error event: {}",
        stderr_of(&delete)
    );
    let create = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_sales",
        "CREATE TABLE hack (a INTEGER)",
    )
    .output()
    .unwrap();
    assert_eq!(
        create.status.code(),
        Some(4),
        "CREATE must be refused, stderr: {}",
        stderr_of(&create)
    );
    let _ = fs::remove_dir_all(&root);
}
