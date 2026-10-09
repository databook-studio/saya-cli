/// Exports `id` from `a` and rewrites its `objects` field to `objects` in a
/// second file, as an editor (or a lying exporter) would hand a document in.
fn exported_with_objects(a: &Harness, id: &str, objects: serde_json::Value) -> PathBuf {
    let exported = a.root.join("shared.json");
    let out = a.run(&["investigation", "export", id, exported.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "export failed: {}{}",
        a.stderr(&out),
        a.stdout(&out)
    );
    let mut document: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&exported).unwrap()).unwrap();
    document["objects"] = objects;
    let tampered = a.root.join("tampered.json");
    fs::write(&tampered, serde_json::to_string_pretty(&document).unwrap()).unwrap();
    tampered
}

#[test]
fn imported_empty_objects_cannot_skip_schema_review() {
    // The audit repro (A2): a document whose objects list is emptied must not
    // import, because run takes its review dependencies from the SQL and an
    // empty stored list could otherwise make a changed table replay cleanly.
    let a = harness_with_database("empty-objects-a");
    let id = saved_id(&a, "empty-objects-a");
    let tampered = exported_with_objects(&a, &id, serde_json::json!([]));

    let b = harness_with_database("empty-objects-b");
    let imported = b.run(&["investigation", "import", tampered.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(2),
        "the emptied objects list is refused: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    assert!(
        b.stderr(&imported)
            .contains("objects do not match the SQL; re-export the investigation"),
        "err: {}",
        b.stderr(&imported)
    );
    assert!(
        b.document_ids().is_empty(),
        "a refused import stores nothing"
    );
    let _ = fs::remove_dir_all(&a.root);
    let _ = fs::remove_dir_all(&b.root);
}

#[test]
fn imported_wrong_objects_are_refused() {
    let a = harness_with_database("wrong-objects-a");
    let id = saved_id(&a, "wrong-objects-a");
    let tampered = exported_with_objects(&a, &id, serde_json::json!(["somewhere.else"]));

    let b = harness_with_database("wrong-objects-b");
    let imported = b.run(&["investigation", "import", tampered.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(2),
        "objects that disagree with the SQL are refused: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    assert!(
        b.stderr(&imported)
            .contains("objects do not match the SQL; re-export the investigation"),
        "err: {}",
        b.stderr(&imported)
    );
    assert!(b.document_ids().is_empty());
    let _ = fs::remove_dir_all(&a.root);
    let _ = fs::remove_dir_all(&b.root);
}

#[test]
fn export_refuses_existing_destination() {
    let a = harness_with_database("export-exists");
    let id = saved_id(&a, "export-exists");
    let dest = a.root.join("shared.json");
    let first = a.run(&["investigation", "export", &id, dest.to_str().unwrap()]);
    assert_eq!(first.status.code(), Some(0), "{}", a.stderr(&first));
    let exported = fs::read(&dest).unwrap();

    let second = a.run(&["investigation", "export", &id, dest.to_str().unwrap()]);
    assert_eq!(
        second.status.code(),
        Some(2),
        "an existing destination is refused: {}{}",
        a.stderr(&second),
        a.stdout(&second)
    );
    assert!(
        a.stderr(&second).contains("exists; pass --overwrite"),
        "err: {}",
        a.stderr(&second)
    );
    assert_eq!(
        fs::read(&dest).unwrap(),
        exported,
        "the refusal preserves the destination"
    );

    let over = a.run(&[
        "investigation",
        "export",
        &id,
        dest.to_str().unwrap(),
        "--overwrite",
    ]);
    assert_eq!(
        over.status.code(),
        Some(0),
        "--overwrite replaces: {}{}",
        a.stderr(&over),
        a.stdout(&over)
    );
    let _ = fs::remove_dir_all(&a.root);
}

#[test]
fn failed_export_preserves_destination() {
    let a = harness_with_database("export-failed");
    let id = saved_id(&a, "export-failed");

    // A directory destination is refused; its contents stay untouched and
    // no staging temp file remains anywhere in it.
    let outdir = a.root.join("outdir");
    fs::create_dir_all(&outdir).unwrap();
    fs::write(outdir.join("keep.txt"), b"keep").unwrap();
    let refused = a.run(&["investigation", "export", &id, outdir.to_str().unwrap()]);
    assert_eq!(
        refused.status.code(),
        Some(2),
        "a directory destination is refused: {}{}",
        a.stderr(&refused),
        a.stdout(&refused)
    );
    assert!(
        a.stderr(&refused).contains("is a directory"),
        "err: {}",
        a.stderr(&refused)
    );
    assert!(outdir.join("keep.txt").exists(), "directory unchanged");
    assert!(list_tmp_files(&outdir).is_empty(), "no temp file remains");

    // A symlink destination is refused even though its target is writable.
    let link = a.root.join("linked.json");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outdir.join("keep.txt"), &link).unwrap();
    let refused = a.run(&["investigation", "export", &id, link.to_str().unwrap()]);
    assert_eq!(
        refused.status.code(),
        Some(2),
        "a symlink destination is refused: {}{}",
        a.stderr(&refused),
        a.stdout(&refused)
    );
    assert!(
        a.stderr(&refused).contains("symlink"),
        "err: {}",
        a.stderr(&refused)
    );

    // An I/O failure (parent directory missing) leaves nothing behind and
    // refuses with exit 2.
    let missing = a.root.join("nosuchdir").join("out.json");
    let failed = a.run(&["investigation", "export", &id, missing.to_str().unwrap()]);
    assert_eq!(
        failed.status.code(),
        Some(2),
        "an unwritable destination is refused: {}{}",
        a.stderr(&failed),
        a.stdout(&failed)
    );
    assert!(!a.root.join("nosuchdir").exists(), "nothing was created");
    assert!(list_tmp_files(&a.root).is_empty(), "no temp file remains");

    // A missing id is refused before any path is touched.
    let refused = a.run(&[
        "investigation",
        "export",
        "no-such-id-00000000",
        a.root.join("x.json").to_str().unwrap(),
    ]);
    assert_eq!(refused.status.code(), Some(2), "{}", a.stderr(&refused));
    assert!(
        a.stderr(&refused)
            .contains("no investigation no-such-id-00000000"),
        "err: {}",
        a.stderr(&refused)
    );
    let _ = fs::remove_dir_all(&a.root);
}
