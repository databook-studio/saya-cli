#[tokio::test]
async fn investigation_replay_obeys_read_only_gate() {
    let h = harness_with_database("read-only");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);

    // The definition is rewritten directly on disk, bypassing save's gate.
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    fs::write(
        h.document_path(&id),
        document.replace(
            "SELECT id, label FROM events ORDER BY id",
            "DELETE FROM events",
        ),
    )
    .unwrap();

    let output = h.run(&["investigation", "run", &id]);
    assert_eq!(
        output.status.code(),
        Some(4),
        "the safety gate refuses the replay: {}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    assert!(
        h.stderr(&output).contains("read-only"),
        "the refusal names the gate: {}",
        h.stderr(&output)
    );
    assert_eq!(
        event_rows(&h.database).await,
        2,
        "the table is intact: nothing executed"
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn first_explicit_mapping_creates_the_binding() {
    let h = harness_with_database("first-mapping");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);
    fs::remove_file(h.binding_path(&id)).unwrap();

    // No binding: run refuses without an explicit target.
    let refused = h.run(&["investigation", "run", &id]);
    assert_eq!(refused.status.code(), Some(2), "{}", h.stderr(&refused));

    // The explicit mapping runs and creates the binding.
    let mapped = h.run(&["investigation", "run", &id, "--connection", "local"]);
    assert_eq!(
        mapped.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&mapped),
        h.stderr(&mapped)
    );
    let stdout = h.stdout(&mapped);
    assert!(
        stdout.contains("saved investigation: local"),
        "the evidence line names the profile: {stdout}"
    );
    assert!(
        h.binding_path(&id).exists(),
        "the mapping created the binding"
    );

    // After the binding exists, the plain run resolves its target from it.
    let plain = h.run(&["investigation", "run", &id]);
    assert_eq!(
        plain.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&plain),
        h.stderr(&plain)
    );
    let _ = fs::remove_dir_all(h.root);
}

#[tokio::test]
async fn run_refuses_missing_malformed_and_stale_revisions() {
    let h = harness_with_database("refusals");
    seed_events(&h.database).await;
    let id = saved_id(&h, None);

    let malformed = h.run(&["investigation", "run", "NOT AN ID"]);
    assert_eq!(malformed.status.code(), Some(2), "{}", h.stderr(&malformed));
    assert!(
        h.stderr(&malformed).contains("no investigation"),
        "err: {}",
        h.stderr(&malformed)
    );

    let missing = h.run(&["investigation", "run", "ghost-00000000"]);
    assert_eq!(missing.status.code(), Some(2), "{}", h.stderr(&missing));
    assert!(
        h.stderr(&missing)
            .contains("no investigation ghost-00000000"),
        "err: {}",
        h.stderr(&missing)
    );

    // A definition revised on disk (revision 2, binding says 1) is stale.
    let document = fs::read_to_string(h.document_path(&id)).unwrap();
    fs::write(
        h.document_path(&id),
        document.replace("\"revision\": 1", "\"revision\": 2"),
    )
    .unwrap();
    let stale = h.run(&["investigation", "run", &id]);
    assert_eq!(stale.status.code(), Some(2), "{}", h.stderr(&stale));
    let stderr = h.stderr(&stale);
    assert!(
        stderr.contains("revision changed"),
        "the refusal names what changed: {stderr}"
    );
    assert!(
        !h.stdout(&stale).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&stale)
    );
    let _ = fs::remove_dir_all(h.root);
}
