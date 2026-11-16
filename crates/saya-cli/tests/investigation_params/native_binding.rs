/// A parameterized replay binds natively on SQLite and returns the filtered
/// rows; the parameters ride `QueryRequest::with_params`, not the SQL text.
#[tokio::test]
async fn parameterized_replay_binds_natively_on_sqlite() {
    let h = harness_with_database("sqlite-pass");
    seed_events(&h.database).await;
    let id = h.saved_id(
        "By label",
        "SELECT id FROM events WHERE label = :label ORDER BY id",
        &["label:string"],
    );

    let output = h.run(&["investigation", "run", &id, "--param", "label=second"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stdout = h.stdout(&output);
    assert!(
        stdout.contains("\"event\":\"query_result\""),
        "the replay executed: {stdout}"
    );
    assert!(
        stdout.contains("\"row_count\":1") && stdout.contains("\"rows\":[[2]]"),
        "exactly the bound row came back: {stdout}"
    );
    let _ = fs::remove_dir_all(h.root);
}

/// Capability honesty: a SQLite pass (native binds, no server needed), and
/// a ClickHouse-dialect definition refused honestly before any connection
/// attempt (the profile points nowhere). The ClickHouse definition is
/// constructed directly on disk — ClickHouse cannot be reached from here.
#[tokio::test]
async fn unsupported_binding_capability_is_honest() {
    let local_database = label_database("capability-local");
    fs::write(&local_database, b"").unwrap();
    let h = harness(
        "capability",
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n\n\
             [profiles.ch]\ntype = 'clickhouse'\nhost = '127.0.0.1'\nport = 1\ndatabase = 'db'\nuser = 'u'\n",
            local_database.display()
        ),
    );
    seed_events(&local_database).await;

    // The SQLite pass: the parameterized replay binds natively.
    let id = h.saved_id_for(
        "By label",
        "SELECT id FROM events WHERE label = :label ORDER BY id",
        &["label:string"],
        Some("local"),
    );
    let pass = h.run(&[
        "investigation",
        "run",
        &id,
        "--connection",
        "local",
        "--param",
        "label=first",
    ]);
    assert_eq!(
        pass.status.code(),
        Some(0),
        "SQLite binds natively: {}{}",
        h.stdout(&pass),
        h.stderr(&pass)
    );
    assert!(
        h.stdout(&pass).contains("\"rows\":[[1]]"),
        "exactly the bound row came back: {}",
        h.stdout(&pass)
    );

    // The ClickHouse-dialect definition, written directly as the task allows:
    // its placeholders are declared, but no ClickHouse server is reachable,
    // so any connect attempt would fail with a connection error — the honest
    // capability refusal must win first.
    let id = "by-region-01234567";
    fs::write(
        h.document_path(id),
        r#"{
  "format": "saya.investigation",
  "version": 1,
  "id": "by-region-01234567",
  "revision": 1,
  "name": "By region",
  "sql": "SELECT 1 WHERE x = :region",
  "parameters": [{"name": "region", "type": "string", "required": false}],
  "dialect": "clickhouse",
  "connection": "ch",
  "objects": [],
  "schema_fingerprint": null,
  "created_unix_ms": 1700000000000,
  "updated_unix_ms": 1700000000000
}"#,
    )
    .unwrap();

    let output = h.run(&[
        "investigation",
        "run",
        id,
        "--connection",
        "ch",
        "--param",
        "region=east",
    ]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "the unsupported engine refuses: {}{}",
        h.stdout(&output),
        h.stderr(&output)
    );
    let stderr = h.stderr(&output);
    assert!(
        stderr.contains("parameters are not supported for ClickHouse"),
        "the refusal names the engine honestly: {stderr}"
    );
    assert!(
        !h.stdout(&output).contains("\"event\":\"query_result\""),
        "nothing executed: {}",
        h.stdout(&output)
    );
    let _ = fs::remove_dir_all(h.root);
    let _ = fs::remove_dir_all(local_database.parent().unwrap());
}
