/// A922-4 reproduction: opening the same content with `--no-header` must not
/// reuse the header snapshot — the second preview (2 rows, column_1/column_2)
/// must equal what a query through the second snapshot's profile returns.
#[test]
fn no_header_change_stages_a_separate_snapshot_whose_preview_matches_the_query() {
    let root = isolated_root("no-header");
    let content = "zip,city\n02134,Boston\n";
    let csv = root.join("a.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let first = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    let first_out = stdout_of(&first);
    assert!(first_out.contains("Rows: 1"), "{first_out}");
    assert!(first_out.contains("zip: text"), "{first_out}");
    assert!(first_out.contains("city: text"), "{first_out}");

    let second = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--no-header",
        ])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0), "{}", stderr_of(&second));
    let second_out = stdout_of(&second);
    assert!(
        second_out.contains("(staged now)"),
        "a changed header flag must stage a fresh snapshot, not reuse: {second_out}"
    );
    assert!(second_out.contains("Rows: 2"), "{second_out}");
    assert!(second_out.contains("column_1: text"), "{second_out}");
    assert!(second_out.contains("column_2: text"), "{second_out}");

    let dirs = snapshot_dirs(&root, &sha);
    assert_eq!(
        dirs.len(),
        2,
        "two snapshots of the same content: {:?}",
        dirs
    );
    for dir in &dirs {
        let name = dir.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with(&sha[..16]),
            "the name keeps the content-hash prefix first: {name}"
        );
    }
    assert_preview_matches_query(&root, &first_out, "a");
    assert_preview_matches_query(&root, &second_out, "a");
    // The wrong spelling fails in each direction: the header snapshot has no
    // column_1, the no-header one has no zip.
    let (first_connections, first_profile) =
        (preview_connections(&first_out), preview_profile(&first_out));
    let (second_connections, second_profile) = (
        preview_connections(&second_out),
        preview_profile(&second_out),
    );
    column_query_fails(
        &root,
        &first_connections,
        &first_profile,
        "SELECT column_1 FROM a LIMIT 1",
    );
    column_query_fails(
        &root,
        &second_connections,
        &second_profile,
        "SELECT zip FROM a LIMIT 1",
    );
    let _ = fs::remove_dir_all(&root);
}

/// A changed delimiter is a different contract: its own snapshot, and the
/// preview equals the query through that snapshot's profile.
#[test]
fn delimiter_change_stages_a_separate_snapshot_whose_preview_matches_the_query() {
    let root = isolated_root("delimiter");
    let content = "x;y\n1;2\n";
    let csv = root.join("d.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let first = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--delimiter",
            ";",
        ])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    let first_out = stdout_of(&first);
    assert!(first_out.contains("Rows: 1"), "{first_out}");
    assert!(first_out.contains("x: integer"), "{first_out}");

    let second = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--delimiter",
            ",",
            "--no-header",
        ])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0), "{}", stderr_of(&second));
    let second_out = stdout_of(&second);
    assert!(
        second_out.contains("(staged now)"),
        "a changed delimiter must stage a fresh snapshot, not reuse: {second_out}"
    );
    assert!(second_out.contains("Rows: 2"), "{second_out}");
    assert!(second_out.contains("column_1: text"), "{second_out}");

    let dirs = snapshot_dirs(&root, &sha);
    assert_eq!(
        dirs.len(),
        2,
        "two snapshots of the same content: {:?}",
        dirs
    );
    assert_preview_matches_query(&root, &first_out, "d");
    assert_preview_matches_query(&root, &second_out, "d");
    let _ = fs::remove_dir_all(&root);
}

/// The audit's renamed-file repro: identical content under a new name opened
/// with `--typed` stages its own snapshot (different table name and typed
/// flag in the contract) and the typed copy works — instead of the old
/// exit-2 `could not build the typed copy "b_typed"` against a foreign table.
#[test]
fn renamed_identical_file_with_typed_stages_its_own_snapshot() {
    let root = isolated_root("renamed-typed");
    let content = "zip,city\n02134,Boston\n";
    let a = root.join("a.csv");
    fs::write(&a, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let first = open_command(&root)
        .args(["open", a.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    let first_out = stdout_of(&first);

    let b = root.join("b.csv");
    fs::copy(&a, &b).unwrap();
    let second = open_command(&root)
        .args(["open", b.to_str().unwrap(), "--non-interactive", "--typed"])
        .output()
        .unwrap();
    assert_eq!(
        second.status.code(),
        Some(0),
        "the renamed --typed open must succeed: {} {}",
        stdout_of(&second),
        stderr_of(&second)
    );
    let second_out = stdout_of(&second);
    assert!(second_out.contains("File: b.csv"), "{second_out}");
    assert!(second_out.contains("Typed copy: b_typed"), "{second_out}");

    assert_eq!(
        snapshot_dirs(&root, &sha).len(),
        2,
        "table name and typed flag separate the snapshots"
    );
    assert_preview_matches_query(&root, &first_out, "a");
    assert_preview_matches_query(&root, &second_out, "b");
    let b_connections = preview_connections(&second_out);
    let typed = query_result(
        &root,
        &b_connections,
        "file_b",
        "SELECT zip FROM b_typed LIMIT 1",
    );
    assert_eq!(
        typed["result"]["rows"][0][0].as_str(),
        Some("02134"),
        "the typed copy holds the staged values: {typed}"
    );
    let _ = fs::remove_dir_all(&root);
}
