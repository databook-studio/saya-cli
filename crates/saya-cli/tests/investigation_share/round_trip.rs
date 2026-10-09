#[test]
fn export_then_import_into_a_clean_root() {
    let a = harness_with_database("share-a");
    let id = saved_id(&a, "share-a");

    let exported = a.root.join("shared.json");
    let out = a.run(&["investigation", "export", &id, exported.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "export failed: {}{}",
        a.stderr(&out),
        a.stdout(&out)
    );

    // The exported file is the portable definition only.
    let exported_text = fs::read_to_string(&exported).unwrap();
    assert!(exported_text.contains("SELECT id, label FROM events ORDER BY id"));
    for forbidden in ["profile_identity", "local binding", "password"] {
        assert!(
            !exported_text.contains(forbidden),
            "export must not carry {forbidden:?}: {exported_text}"
        );
    }

    // A clean second root: import, then show reports no binding and the
    // identical definition.
    let b = harness_with_database("share-b");
    let imported = b.run(&["investigation", "import", exported.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(0),
        "import failed: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    let preview = b.stdout(&imported);
    assert!(
        preview.contains(
            "Imported without a local connection. Run with --connection <profile> to map it; nothing was executed."
        ),
        "preview: {preview}"
    );
    assert!(preview.contains(&id), "preview names the id: {preview}");
    assert!(
        preview.contains("Order events"),
        "preview names the investigation: {preview}"
    );
    assert!(
        preview.contains("SELECT id, label FROM events ORDER BY id"),
        "preview shows the exact SQL: {preview}"
    );
    assert!(
        preview.contains("events"),
        "preview lists objects: {preview}"
    );

    // The stored document is the exported bytes, exactly.
    let stored = fs::read(b.document_path(&id)).unwrap();
    assert_eq!(
        stored,
        fs::read(&exported).unwrap(),
        "import must store the exported bytes exactly"
    );

    assert_show_matches_export(&b, &id, &exported, "local binding: none");
    // And in the exporting root the binding is still recorded there.
    assert!(
        a.binding_path(&id).exists(),
        "export left A's binding alone"
    );
}

#[test]
fn import_never_executes_or_grants() {
    // The profile names a database path that does not exist: any connection
    // or query attempt would create it, so its absence proves import never
    // connected.
    let b = harness(
        "no-exec",
        &format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            std::env::temp_dir()
                .join(format!("saya-share-noexec-db-{}", std::process::id()))
                .display()
        ),
    );
    let file = definition_file(&b.root, "order.json", DEFINITION_JSON);
    let imported = b.run(&["investigation", "import", file.to_str().unwrap()]);
    assert_eq!(
        imported.status.code(),
        Some(0),
        "import failed: {}{}",
        b.stderr(&imported),
        b.stdout(&imported)
    );
    let db_path = std::env::temp_dir().join(format!("saya-share-noexec-db-{}", std::process::id()));
    assert!(
        !db_path.exists(),
        "import must never connect to the profile's database"
    );
    assert!(
        !b.investigations.join("local").exists(),
        "import must never write a binding"
    );

    let id = "order-events-01234567";
    let show = b.run(&["investigation", "show", id]);
    assert_eq!(show.status.code(), Some(0), "{}", b.stderr(&show));
    assert!(
        b.stdout(&show).contains("local binding: none"),
        "imported without a binding: {}",
        b.stdout(&show)
    );
    let _ = fs::remove_dir_all(&b.root);
}
