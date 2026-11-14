/// Seeds the `events` table with two rows through sqlx (direct, not through
/// saya), so the post-edit `--revalidate` replay has something to read.
async fn seed_events(database: &std::path::Path) {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("CREATE TABLE events (id INTEGER PRIMARY KEY, label TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO events (id, label) VALUES (1, 'first'), (2, 'second')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
async fn edit_updates_the_document_and_the_next_run_refuses_until_revalidate() {
    let h = harness_with_database("edit-arc");
    seed_events(&label_database("edit-arc")).await;
    let id = saved_id(&h);

    let edit = h.run(&[
        "investigation",
        "edit",
        &id,
        "--sql",
        "SELECT label FROM events WHERE id = 2",
    ]);
    assert_eq!(
        edit.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&edit),
        h.stderr(&edit)
    );
    let stdout = h.stdout(&edit);
    assert!(
        stdout.contains("Edited to revision 2. The next run needs --revalidate."),
        "{stdout}"
    );

    let show = h.run(&["investigation", "show", &id]);
    assert_eq!(show.status.code(), Some(0), "{}", h.stderr(&show));
    let show_stdout = h.stdout(&show);
    assert!(show_stdout.contains("SELECT label FROM events WHERE id = 2"));
    assert!(
        show_stdout.contains("local binding: local (reviewed revision 1)"),
        "the binding stays on the old revision: {show_stdout}"
    );

    // The stale review refuses the next run; --revalidate accepts it.
    let stale = h.run(&["investigation", "run", &id]);
    assert_eq!(stale.status.code(), Some(2), "{}", h.stderr(&stale));
    assert!(
        h.stderr(&stale).contains("revision changed"),
        "err: {}",
        h.stderr(&stale)
    );
    let revalidated = h.run(&["investigation", "run", &id, "--revalidate"]);
    assert_eq!(
        revalidated.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&revalidated),
        h.stderr(&revalidated)
    );

    // The list shows the advanced revision under the unchanged name.
    let list = h.run(&["investigation", "list"]);
    assert_eq!(list.status.code(), Some(0));
    assert!(
        h.stdout(&list)
            .contains(&format!("{id}  2  sqlite  local  Order events")),
        "list line: {}",
        h.stdout(&list)
    );
    let _ = fs::remove_dir_all(&h.root);
}

#[test]
fn edit_refuses_write_sql_and_changes_nothing() {
    let h = harness_with_database("edit-write-sql");
    let id = saved_id(&h);
    let document_before = fs::read_to_string(h.document_path(&id)).unwrap();
    let edit = h.run(&["investigation", "edit", &id, "--sql", "DELETE FROM events"]);
    assert_eq!(edit.status.code(), Some(4), "{}", h.stderr(&edit));
    assert!(
        h.stderr(&edit).contains("read-only safety policy"),
        "err: {}",
        h.stderr(&edit)
    );
    assert_eq!(
        fs::read_to_string(h.document_path(&id)).unwrap(),
        document_before,
        "a refused edit changes nothing"
    );
    let _ = fs::remove_dir_all(&h.root);
}

#[test]
fn edit_with_no_fields_is_a_usage_error() {
    let h = harness_with_database("edit-no-fields");
    let id = saved_id(&h);
    let edit = h.run(&["investigation", "edit", &id]);
    assert_eq!(edit.status.code(), Some(2), "{}", h.stderr(&edit));
    let stderr = h.stderr(&edit);
    assert!(
        stderr.contains("edit needs a change") && stderr.contains("--sql"),
        "the refusal names the flags: {stderr}"
    );
    let _ = fs::remove_dir_all(&h.root);
}

#[test]
fn edit_conflict_when_the_document_moved_underneath() {
    let h = harness_with_database("edit-conflict");
    let id = saved_id(&h);
    // Another editor wins the race: the document is parked at the maximum
    // revision, which the store deterministically refuses to advance —
    // the same optimistic-conflict refusal a stale expected revision gets.
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    fs::write(
        h.document_path(&id),
        document.replace("\"revision\": 1", "\"revision\": 4294967295"),
    )
    .unwrap();
    let edit = h.run(&["investigation", "edit", &id, "--name", "Renamed"]);
    assert_eq!(edit.status.code(), Some(2), "{}", h.stderr(&edit));
    assert!(
        h.stderr(&edit)
            .contains("changed underneath this command; retry"),
        "err: {}",
        h.stderr(&edit)
    );
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    assert!(document.contains("\"revision\": 4294967295"), "{document}");
    assert!(
        document.contains("Order events"),
        "nothing changed: {document}"
    );
    let _ = fs::remove_dir_all(&h.root);
}
