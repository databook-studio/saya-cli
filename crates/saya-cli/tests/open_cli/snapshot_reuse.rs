#[test]
fn same_content_reuses_the_staged_snapshot() {
    let root = isolated_root("reuse");
    let content = "id,name\n1,one\n";
    let csv = root.join("sales.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let first = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    let db = snapshot_dir(&root, &sha).join("source.duckdb");
    let before = fs::metadata(&db).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(50));

    let second = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0), "{}", stderr_of(&second));
    assert!(
        stdout_of(&second).contains("(reused existing snapshot)"),
        "the second open states the reuse: {}",
        stdout_of(&second)
    );
    assert_eq!(
        fs::metadata(&db).unwrap().modified().unwrap(),
        before,
        "reuse must not rewrite the staged snapshot"
    );
    let staged_count = fs::read_dir(root.join("files")).unwrap().count();
    assert_eq!(staged_count, 1, "reuse creates no second snapshot");

    let reset = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--reset",
        ])
        .output()
        .unwrap();
    assert_eq!(reset.status.code(), Some(0), "{}", stderr_of(&reset));
    assert_ne!(
        fs::metadata(&db).unwrap().modified().unwrap(),
        before,
        "--reset restages into the same snapshot directory"
    );
    let _ = fs::remove_dir_all(&root);
}
