/// Saving SQL whose `:name` placeholders are not declared refuses, naming
/// them; declaring a parameter the SQL does not use refuses too.
#[tokio::test]
async fn save_refuses_undeclared_placeholders_in_both_directions() {
    let h = harness_with_database("undeclared");
    seed_events(&h.database).await;

    let undeclared = h.run(&[
        "investigation",
        "save",
        "--name",
        "Missing spec",
        "--sql",
        "SELECT id FROM events WHERE label = :label",
    ]);
    assert_eq!(
        undeclared.status.code(),
        Some(2),
        "{}{}",
        h.stdout(&undeclared),
        h.stderr(&undeclared)
    );
    let stderr = h.stderr(&undeclared);
    assert!(
        stderr.contains("label") && stderr.contains("--param-spec"),
        "the refusal names the placeholder and the declaring flag: {stderr}"
    );

    let extra = h.run(&[
        "investigation",
        "save",
        "--name",
        "Extra spec",
        "--sql",
        "SELECT id FROM events WHERE label = :label",
        "--param-spec",
        "region:string",
    ]);
    assert_eq!(
        extra.status.code(),
        Some(2),
        "{}{}",
        h.stdout(&extra),
        h.stderr(&extra)
    );
    let stderr = h.stderr(&extra);
    assert!(
        stderr.contains("label") && stderr.contains("region"),
        "the refusal names both the missing and the extra name: {stderr}"
    );

    // A malformed spec is a usage refusal, not a save.
    let bad_type = h.run(&[
        "investigation",
        "save",
        "--name",
        "Bad type",
        "--sql",
        "SELECT :label",
        "--param-spec",
        "label:str",
    ]);
    assert_eq!(bad_type.status.code(), Some(2), "{}", h.stderr(&bad_type));
    let stderr = h.stderr(&bad_type);
    assert!(
        stderr.contains("string|integer|boolean|decimal|date|timestamp"),
        "the refusal names the valid types: {stderr}"
    );
    let _ = fs::remove_dir_all(h.root);
}

/// A save with matching declarations stores them in the definition; the run
/// binds them.
#[tokio::test]
async fn save_stores_the_declared_parameters() {
    let h = harness_with_database("declared");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "Since day",
        "SELECT id FROM events WHERE id > :since AND label = :label",
        &["since:date", "label:string:required"],
    );

    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    assert!(
        document.contains("\"parameters\"")
            && document.contains("\"type\": \"date\"")
            && document.contains("\"required\": true"),
        "the declared parameters persist in the definition: {document}"
    );
    assert!(
        !document.contains("params_sha256"),
        "a definition never carries a value digest: {document}"
    );
    let _ = fs::remove_dir_all(h.root);
}

// ---------------------------------------------------------------------------
// invariant 2: edit replaces the spec list, bumps the revision, stales
// ---------------------------------------------------------------------------

#[tokio::test]
async fn edit_replaces_the_spec_bumps_the_revision_and_stales() {
    let h = harness_with_database("edit-spec");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "By label",
        "SELECT id FROM events WHERE label = :label",
        &["label:string"],
    );

    // Editing only the spec list is a valid edit: the placeholders must keep
    // matching the SQL.
    let edited = h.run(&[
        "investigation",
        "edit",
        &id,
        "--param-spec",
        "label:string:required",
    ]);
    assert_eq!(
        edited.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&edited),
        h.stderr(&edited)
    );
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    assert!(
        document.contains("\"revision\": 2") && document.contains("\"required\": true"),
        "the edit published revision 2 with the replaced spec: {document}"
    );

    // The spec change is a new revision, so the review is stale.
    let stale = h.run(&["investigation", "run", &id, "--param", "label=first"]);
    assert_eq!(stale.status.code(), Some(2), "{}", h.stderr(&stale));
    let stderr = h.stderr(&stale);
    assert!(
        stderr.contains("revision changed"),
        "the stale refusal names what changed: {stderr}"
    );

    // An edit whose specs no longer cover the SQL refuses, naming the
    // placeholder.
    let uncovered = h.run(&["investigation", "edit", &id, "--param-spec", "day:date"]);
    assert_eq!(
        uncovered.status.code(),
        Some(2),
        "{}{}",
        h.stdout(&uncovered),
        h.stderr(&uncovered)
    );
    let stderr = h.stderr(&uncovered);
    assert!(
        stderr.contains("label"),
        "the refusal names the now-undeclared placeholder: {stderr}"
    );

    let revalidated = h.run(&[
        "investigation",
        "run",
        &id,
        "--param",
        "label=first",
        "--revalidate",
    ]);
    assert_eq!(
        revalidated.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&revalidated),
        h.stderr(&revalidated)
    );
    let _ = fs::remove_dir_all(h.root);
}
