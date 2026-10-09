/// An explicitly typed null (`--param name=null`) and an omitted optional
/// parameter both bind as nulls on SQLite, and the run succeeds.
#[tokio::test]
async fn typed_null_binds_on_sqlite() {
    let h = harness_with_database("typed-null");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "Null probes",
        "SELECT :v IS NULL AS v_null, :w IS NULL AS w_null",
        &["v:date", "w:string:required"],
    );

    let explicit = h.run(&[
        "investigation",
        "run",
        &id,
        "--param",
        "v=null",
        "--param",
        "w=null",
    ]);
    assert_eq!(
        explicit.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&explicit),
        h.stderr(&explicit)
    );
    let stdout = h.stdout(&explicit);
    assert!(
        stdout.contains("\"rows\":[[1,1]]"),
        "both typed nulls bound: {stdout}"
    );

    // An optional parameter left out binds as a typed null too; the supplied
    // one binds as its value.
    let omitted = h.run(&["investigation", "run", &id, "--param", "w=first"]);
    assert_eq!(
        omitted.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&omitted),
        h.stderr(&omitted)
    );
    assert!(
        h.stdout(&omitted).contains("\"rows\":[[1,0]]"),
        "the omitted optional parameter bound as a typed null: {}",
        h.stdout(&omitted)
    );
    let _ = fs::remove_dir_all(h.root);
}

// ---------------------------------------------------------------------------
// invariant 1: import validates the placeholder/declaration contract too
// ---------------------------------------------------------------------------

/// An imported document whose placeholders and declarations disagree is
/// refused, naming them — a hand-written file gets the same gate save does.
#[tokio::test]
async fn import_refuses_a_placeholder_declaration_mismatch() {
    let h = harness_with_database("import-contract");
    seed_events(&h.database).await;

    let mismatched = r#"{
  "format": "saya.investigation",
  "version": 1,
  "id": "mismatched-01234567",
  "revision": 1,
  "name": "Mismatched",
  "sql": "SELECT id FROM events WHERE label = :label",
  "parameters": [{"name": "region", "type": "string", "required": false}],
  "dialect": "sqlite",
  "connection": "local",
  "objects": ["events"],
  "schema_fingerprint": null,
  "created_unix_ms": 1700000000000,
  "updated_unix_ms": 1700000000000
}"#;
    let path = h.root.join("mismatched.json");
    fs::write(&path, mismatched).unwrap();

    let output = h.run(&["investigation", "import", path.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stderr = h.stderr(&output);
    assert!(
        stderr.contains("label") && stderr.contains("region"),
        "the refusal names the missing and the extra name: {stderr}"
    );

    // A matching document imports.
    let matching = r#"{
  "format": "saya.investigation",
  "version": 1,
  "id": "matched-01234567",
  "revision": 1,
  "name": "Matched",
  "sql": "SELECT id FROM events WHERE label = :label",
  "parameters": [{"name": "label", "type": "string", "required": false}],
  "dialect": "sqlite",
  "connection": "local",
  "objects": ["events"],
  "schema_fingerprint": null,
  "created_unix_ms": 1700000000000,
  "updated_unix_ms": 1700000000000
}"#;
    let path = h.root.join("matched.json");
    fs::write(&path, matching).unwrap();
    let output = h.run(&["investigation", "import", path.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let _ = fs::remove_dir_all(h.root);
}
