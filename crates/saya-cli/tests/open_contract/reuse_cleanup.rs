/// Plain reuse survives: an identical invocation — and one whose explicit
/// delimiter only names the delimiter the sniff would pick anyway — reuses
/// the snapshot without restaging it.
#[test]
fn identical_invocation_still_reuses_without_restage() {
    let root = isolated_root("reuse");
    let content = "x;y\n1;2\n";
    let csv = root.join("r.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    let first = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    let first_out = stdout_of(&first);
    assert!(
        first_out.contains("(staged now)"),
        "the first open stages: {first_out}"
    );
    let connections = preview_connections(&first_out);
    let db = connections.parent().unwrap().join("source.duckdb");
    let before = fs::metadata(&db).unwrap().modified().unwrap();

    let second = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0), "{}", stderr_of(&second));
    assert!(
        stdout_of(&second).contains("(reused existing snapshot)"),
        "the identical invocation reuses: {}",
        stdout_of(&second)
    );
    assert_eq!(
        snapshot_dirs(&root, &sha).len(),
        1,
        "reuse creates no second snapshot"
    );

    // An explicit delimiter that names what the sniff would pick resolves to
    // the same effective contract, so it reuses too.
    let third = open_command(&root)
        .args([
            "open",
            csv.to_str().unwrap(),
            "--non-interactive",
            "--delimiter",
            ";",
        ])
        .output()
        .unwrap();
    assert_eq!(third.status.code(), Some(0), "{}", stderr_of(&third));
    assert!(
        stdout_of(&third).contains("(reused existing snapshot)"),
        "the effective contract matches, so it reuses: {}",
        stdout_of(&third)
    );
    assert_eq!(
        fs::metadata(&db).unwrap().modified().unwrap(),
        before,
        "reuse must not rewrite the staged snapshot"
    );
    let _ = fs::remove_dir_all(&root);
}

/// `--cleanup <prefix>` over two snapshots of the same content is refused as
/// ambiguous; `all` removes both.
#[test]
fn cleanup_prefix_over_two_snapshots_of_one_content_is_refused_and_all_removes_both() {
    let root = isolated_root("cleanup-ambiguous");
    let content = "zip,city\n02134,Boston\n";
    let csv = root.join("a.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    for flags in [
        vec!["--non-interactive"],
        vec!["--non-interactive", "--no-header"],
    ] {
        let out = open_command(&root)
            .arg("open")
            .arg(csv.to_str().unwrap())
            .args(&flags)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    }
    assert_eq!(snapshot_dirs(&root, &sha).len(), 2);

    let refused = open_command(&root)
        .args(["open", "--cleanup", &sha[..12], "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(
        refused.status.code(),
        Some(2),
        "a prefix matching two snapshots of one content is ambiguous: {} {}",
        stdout_of(&refused),
        stderr_of(&refused)
    );
    assert!(
        stderr_of(&refused).contains("matches 2 staged sources"),
        "the refusal names the ambiguity: {}",
        stderr_of(&refused)
    );
    assert!(
        stderr_of(&refused).contains("differ only in parse options"),
        "same-content matches cannot be lengthened apart, so the refusal says so: {}",
        stderr_of(&refused)
    );
    assert_eq!(
        snapshot_dirs(&root, &sha).len(),
        2,
        "the ambiguous cleanup removes nothing"
    );

    let all = open_command(&root)
        .args(["open", "--cleanup", "all", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(all.status.code(), Some(0), "{}", stderr_of(&all));
    let all_out = stdout_of(&all);
    assert_eq!(
        all_out.matches("Removed:").count(),
        2,
        "`all` removes both snapshots: {all_out}"
    );
    assert!(
        snapshot_dirs(&root, &sha).is_empty(),
        "both snapshots are gone"
    );
    let _ = fs::remove_dir_all(&root);
}

/// A legacy snapshot (content-hash-only directory, no stored contract) is
/// still recognized: `--list` shows it, a matching open never reuses or
/// mutates it (it stages a fresh contract-suffixed snapshot), and
/// `--cleanup all` removes it too.
#[test]
fn legacy_snapshot_is_listed_never_reused_and_still_cleaned() {
    let root = isolated_root("legacy");
    let content = "id\n1\n";
    let csv = root.join("legacy.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());

    // Hand-build the pre-contract snapshot layout: a 16-hex directory whose
    // metadata table carries only the original seven keys.
    let legacy = root.join("files").join(&sha[..16]);
    fs::create_dir_all(&legacy).unwrap();
    {
        let connection = duckdb::Connection::open(legacy.join("source.duckdb")).unwrap();
        connection
            .execute_batch(&format!(
                "CREATE TABLE legacy (id VARCHAR); \
                 INSERT INTO legacy VALUES ('1'); \
                 CREATE TABLE saya_file_source (key VARCHAR, value VARCHAR); \
                 INSERT INTO saya_file_source VALUES \
                 ('file_name', 'legacy.csv'), ('sha256', '{sha}'), ('bytes', '6'), \
                 ('rows', '1'), ('columns', '1'), ('staged_unix_ms', '0'), ('format', 'csv');"
            ))
            .unwrap();
    }
    let legacy_db = legacy.join("source.duckdb");
    let before = fs::metadata(&legacy_db).unwrap().modified().unwrap();

    let list = open_command(&root)
        .args(["open", "--list", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(list.status.code(), Some(0), "{}", stderr_of(&list));
    assert!(
        stdout_of(&list).contains(&sha[..12]),
        "the legacy snapshot is listed: {}",
        stdout_of(&list)
    );

    let opened = open_command(&root)
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(opened.status.code(), Some(0), "{}", stderr_of(&opened));
    assert!(
        stdout_of(&opened).contains("(staged now)"),
        "a legacy snapshot has no stored contract, so it is never reused: {}",
        stdout_of(&opened)
    );
    assert_eq!(
        fs::metadata(&legacy_db).unwrap().modified().unwrap(),
        before,
        "the legacy snapshot is never mutated"
    );
    assert_eq!(
        snapshot_dirs(&root, &sha).len(),
        2,
        "the fresh contract-suffixed snapshot sits beside the legacy one"
    );

    let all = open_command(&root)
        .args(["open", "--cleanup", "all", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(all.status.code(), Some(0), "{}", stderr_of(&all));
    assert_eq!(
        stdout_of(&all).matches("Removed:").count(),
        2,
        "cleanup still reaches the legacy snapshot: {}",
        stdout_of(&all)
    );
    let _ = fs::remove_dir_all(&root);
}
