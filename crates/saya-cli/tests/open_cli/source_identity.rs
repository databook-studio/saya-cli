#[test]
fn leading_zero_ids_stay_text() {
    let root = isolated_root("leading-zero");
    let content = "id,qty\n007,3\n008,5\n";
    let csv = root.join("batch.csv");
    fs::write(&csv, content).unwrap();
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
        stdout.contains("id: text"),
        "leading zeros stay text: {stdout}"
    );
    assert!(stdout.contains("qty: integer"), "{stdout}");
    assert!(
        stdout.contains("kept as text: id"),
        "the typed copy leaves text columns alone: {stdout}"
    );
    assert!(stdout.contains("Cast failures: none"), "{stdout}");

    let first = query_command(
        &root,
        &snapshot_dir(&root, &sha).join("connections.toml"),
        "file_batch",
        "SELECT id FROM batch_typed ORDER BY id",
    )
    .output()
    .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    assert!(
        stdout_of(&first).contains("007"),
        "the typed copy keeps the leading zeros: {}",
        stdout_of(&first)
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn source_change_invalidates_previous_evidence() {
    let root = isolated_root("changed");
    let csv = root.join("ledger.csv");
    let first = "id,amount\n1,10\n";
    fs::write(&csv, first).unwrap();
    let sha_a = sha256_hex(first.as_bytes());
    let open_a = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(open_a.status.code(), Some(0), "{}", stderr_of(&open_a));

    let second = "id,amount\n1,10\n2,20\n";
    fs::write(&csv, second).unwrap();
    let sha_b = sha256_hex(second.as_bytes());
    let open_b = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(open_b.status.code(), Some(0), "{}", stderr_of(&open_b));

    let list = open_command(&root)
        .args(["open", "--list", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(list.status.code(), Some(0), "{}", stderr_of(&list));
    let stdout = stdout_of(&list);
    assert!(
        stdout.contains(&sha_a[..12]),
        "the old snapshot is still listed: {stdout}"
    );
    assert!(
        stdout.contains(&sha_b[..12]),
        "the changed content is a new snapshot: {stdout}"
    );
    assert!(
        snapshot_dirs(&root, &sha_a)
            .first()
            .is_some_and(|dir| dir.join("source.duckdb").exists()),
        "the old snapshot remains until cleanup"
    );
    let _ = fs::remove_dir_all(&root);
}
