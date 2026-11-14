#[test]
fn investigation_help_lists_subcommands() {
    let h = harness_with_database("help");
    let help = h.run(&["investigation", "--help"]);
    assert_eq!(help.status.code(), Some(0), "{}", h.stderr(&help));
    let stdout = h.stdout(&help);
    for expected in ["save", "list", "show", "delete"] {
        assert!(
            stdout.contains(expected),
            "help must list {expected}: {stdout}"
        );
    }
    let save_help = h.run(&["investigation", "save", "--help"]);
    assert_eq!(save_help.status.code(), Some(0), "{}", h.stderr(&save_help));
    let save_stdout = h.stdout(&save_help);
    for expected in ["--sql", "--file", "--connection", "--name", "--description"] {
        assert!(
            save_stdout.contains(expected),
            "save help must list {expected}: {save_stdout}"
        );
    }
}

#[test]
fn save_then_list_and_show_roundtrip() {
    let h = harness_with_database("roundtrip");

    let empty = h.run(&["investigation", "list"]);
    assert_eq!(empty.status.code(), Some(0));
    assert!(h.stdout(&empty).contains("No saved investigations."));

    let save = h.run(&[
        "investigation",
        "save",
        "--name",
        "Order events",
        "--description",
        "events in order",
        "--sql",
        "SELECT id, label FROM events ORDER BY id",
    ]);
    assert_eq!(
        save.status.code(),
        Some(0),
        "save failed: {}{}",
        h.stdout(&save),
        h.stderr(&save)
    );
    let stdout = h.stdout(&save);
    assert!(
        stdout.contains(
            "Saved exactly as shown. Review the SQL before sharing: literals are stored verbatim."
        ),
        "{stdout}"
    );
    let ids = h.document_ids();
    assert_eq!(ids.len(), 1, "exactly one document written: {ids:?}");
    let id = ids[0].clone();
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    assert!(document.contains("SELECT id, label FROM events ORDER BY id"));
    assert!(
        h.binding_path(&id).exists(),
        "binding written beside the document"
    );

    let list = h.run(&["investigation", "list"]);
    assert_eq!(list.status.code(), Some(0));
    assert!(
        h.stdout(&list)
            .contains(&format!("{id}  1  sqlite  local  Order events")),
        "list line: {}",
        h.stdout(&list)
    );

    let show = h.run(&["investigation", "show", &id]);
    assert_eq!(show.status.code(), Some(0));
    let show_stdout = h.stdout(&show);
    assert!(show_stdout.contains("SELECT id, label FROM events ORDER BY id"));
    assert!(
        show_stdout.contains("local binding: local (reviewed revision 1)"),
        "{show_stdout}"
    );
    // The opaque profile identity never reaches the document or the output.
    assert!(!document.contains("profile_identity"), "{document}");
    assert!(!show_stdout.contains("profile_identity"), "{show_stdout}");
}

#[test]
fn save_refuses_write_sql() {
    let h = harness_with_database("write-sql");
    let save = h.run(&[
        "investigation",
        "save",
        "--name",
        "Wipe",
        "--sql",
        "DELETE FROM events",
    ]);
    assert_eq!(save.status.code(), Some(4), "{}", h.stderr(&save));
    assert!(
        h.stderr(&save).contains("read-only safety policy"),
        "the refusal names the safety gate: {}",
        h.stderr(&save)
    );
    assert!(h.document_ids().is_empty(), "nothing written");
}

#[test]
fn save_refuses_credential_shaped_sql() {
    let h = harness_with_database("credential");
    let save = h.run(&[
        "investigation",
        "save",
        "--name",
        "Leak",
        "--sql",
        "SELECT 'password=hunter2'",
    ]);
    assert_eq!(save.status.code(), Some(2), "{}", h.stderr(&save));
    assert!(
        h.stderr(&save).contains("credential-shaped"),
        "err: {}",
        h.stderr(&save)
    );
    assert!(h.document_ids().is_empty(), "nothing written");
}

#[test]
fn save_requires_a_connection() {
    // No profiles at all: nothing resolves, so save must refuse with the
    // flag to pass instead of guessing.
    let h = harness("no-connection", "");
    let save = h.run(&[
        "investigation",
        "save",
        "--name",
        "Lonely",
        "--sql",
        "SELECT 1",
    ]);
    assert_eq!(save.status.code(), Some(2), "{}", h.stderr(&save));
    assert!(
        h.stderr(&save)
            .contains("no connection: pass --connection <profile>"),
        "err: {}",
        h.stderr(&save)
    );
    assert!(h.document_ids().is_empty());
}

#[test]
fn definition_export_excludes_credentials_rows_and_grants() {
    let h = harness_with_database("sentinel");
    let save = h.run(&[
        "investigation",
        "save",
        "--name",
        "Clean events",
        "--description",
        "labelled events",
        "--sql",
        "SELECT id, label FROM events WHERE label IS NOT NULL",
    ]);
    assert_eq!(save.status.code(), Some(0), "{}", h.stderr(&save));
    let id = h.document_ids().into_iter().next().unwrap();
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    for forbidden in [
        "password",
        "token",
        "rows",
        "grant",
        "identity",
        "profile_identity",
    ] {
        assert!(
            !document.contains(forbidden),
            "the portable document must not contain {forbidden:?}: {document}"
        );
    }
    // The binding is per-machine state beside the document, never inside it.
    let binding = fs::read_to_string(h.binding_path(&id)).unwrap();
    assert!(
        binding.contains("\"profile_identity\""),
        "binding: {binding}"
    );
    assert!(!binding.contains("password"), "binding: {binding}");
}

#[test]
fn delete_removes_definition_and_binding() {
    let h = harness_with_database("delete");
    let id = saved_id(&h);

    let stale = h.run(&["investigation", "delete", &id, "--revision", "5"]);
    assert_eq!(stale.status.code(), Some(2), "{}", h.stderr(&stale));
    let stderr = h.stderr(&stale);
    assert!(
        stderr.contains("at revision 1"),
        "the refusal names the current revision: {stderr}"
    );
    assert!(
        h.document_path(&id).exists(),
        "a refused delete changes nothing"
    );

    let deleted = h.run(&["investigation", "delete", &id, "--revision", "1"]);
    assert_eq!(deleted.status.code(), Some(0), "{}", h.stderr(&deleted));
    assert!(!h.document_path(&id).exists(), "document removed");
    assert!(!h.binding_path(&id).exists(), "binding removed too");

    let show = h.run(&["investigation", "show", &id]);
    assert_eq!(show.status.code(), Some(2));
    assert!(
        h.stderr(&show).contains(&format!("no investigation {id}")),
        "err: {}",
        h.stderr(&show)
    );

    let list = h.run(&["investigation", "list"]);
    assert_eq!(list.status.code(), Some(0));
    assert!(h.stdout(&list).contains("No saved investigations."));
}
