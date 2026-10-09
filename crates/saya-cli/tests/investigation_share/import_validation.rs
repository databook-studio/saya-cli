#[test]
fn import_rejects_write_sql() {
    let b = harness_with_database("write-sql");
    let write_doc = DEFINITION_JSON.replace(
        "SELECT id, label FROM events ORDER BY id",
        "DELETE FROM events",
    );
    let file = definition_file(&b.root, "wipe.json", &write_doc);
    let imported = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(4),
        "the safety gate refuses write SQL: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    assert!(
        b.stderr(&imported).contains("read-only safety policy"),
        "err: {}",
        b.stderr(&imported)
    );
    assert!(
        b.document_ids().is_empty(),
        "a refused import stores nothing"
    );
    let _ = fs::remove_dir_all(&b.root);
}

#[test]
fn import_rejects_oversize_and_unknown_version() {
    let b = harness_with_database("oversize");

    let padded = format!("{DEFINITION_JSON}{}", " ".repeat(131_072));
    let file = definition_file(&b.root, "big.json", &padded);
    let imported = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(2),
        "oversize is a usage error: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    assert!(
        b.stderr(&imported).contains("131072"),
        "the refusal names the byte cap: {}",
        b.stderr(&imported)
    );
    assert!(b.document_ids().is_empty());

    let newer = DEFINITION_JSON.replace("\"version\": 1", "\"version\": 2");
    let file = definition_file(&b.root, "newer.json", &newer);
    let imported = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(2),
        "an unknown major version is refused: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    assert!(
        b.stderr(&imported)
            .contains("unsupported investigation version 2"),
        "err: {}",
        b.stderr(&imported)
    );
    assert!(b.document_ids().is_empty());
    let _ = fs::remove_dir_all(&b.root);
}

#[test]
fn import_is_idempotent_for_identical_content() {
    let b = harness_with_database("idempotent");
    let file = definition_file(&b.root, "order.json", DEFINITION_JSON);

    let first = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(first.status.code(), Some(0), "{}", b.stderr(&first));
    let id = "order-events-01234567";
    let stored = fs::read(b.document_path(id)).unwrap();

    let second = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(
        second.status.code(),
        Some(0),
        "identical re-import is a no-op: {}{}",
        b.stderr(&second),
        b.stdout(&second)
    );
    assert!(
        b.stdout(&second).contains("Already present and identical"),
        "the no-op says so: {}",
        b.stdout(&second)
    );
    assert_eq!(b.document_ids(), vec![id.to_string()]);
    assert_eq!(
        fs::read(b.document_path(id)).unwrap(),
        stored,
        "the stored document is unchanged"
    );
    let _ = fs::remove_dir_all(&b.root);
}

#[test]
fn import_conflict_for_same_id_different_content() {
    let b = harness_with_database("conflict");
    let file = definition_file(&b.root, "order.json", DEFINITION_JSON);
    let first = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(first.status.code(), Some(0), "{}", b.stderr(&first));
    let id = "order-events-01234567";
    let stored = fs::read(b.document_path(id)).unwrap();

    let renamed =
        DEFINITION_JSON.replace("\"name\": \"Order events\"", "\"name\": \"Renamed events\"");
    assert_ne!(renamed, DEFINITION_JSON, "mutation sanity");
    let other = definition_file(&b.root, "renamed.json", &renamed);
    let second = b.run(&["investigation", "import", other.to_str().unwrap()]);
    assert_eq!(
        second.status.code(),
        Some(2),
        "same id, different content is a conflict: {}{}",
        b.stderr(&second),
        b.stdout(&second)
    );
    assert!(
        b.stderr(&second)
            .contains("already exists with different content"),
        "err: {}",
        b.stderr(&second)
    );
    assert_eq!(
        fs::read(b.document_path(id)).unwrap(),
        stored,
        "the conflict changes nothing"
    );
    let _ = fs::remove_dir_all(&b.root);
}
