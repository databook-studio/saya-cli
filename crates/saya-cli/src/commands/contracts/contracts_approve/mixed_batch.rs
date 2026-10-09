/// A mixed batch with `--yes` —
/// one candidate confirms (persisted), one is refused with its reason —
/// reports the per-item outcomes and both counts, and exits 0 (a partial
/// batch is a success, never a rollback). The refusal exercised here is the
/// one reachable from a queue read: a candidate whose object the populated
/// cached schema no longer names (`ObjectGone`). The `Dismissed` refusal
/// (`Conflict`) is exercised at the operations layer
/// (`contracts::tests::approve_all_reports_a_mixed_batch_and_persists_the_successes`)
/// — a dismissed item never appears in a queue snapshot, so it can only
/// reach the sweep through the stale-snapshot window `approve_all` models.
#[tokio::test]
async fn mixed_batch_reports_approvals_and_refusals_and_persists() {
    let root = std::env::temp_dir().join(format!(
        "saya-approve-mixed-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let profile = identity();
    // A populated cache naming only `orders` — so a candidate on `orders`
    // confirms and a candidate on `shipments` is refused as gone.
    let live = saya_types::SchemaTree {
        databases: vec![saya_types::Database {
            name: "catalog".into(),
            schemas: vec![saya_types::Schema {
                name: "public".into(),
                tables: vec![saya_types::Table {
                    name: "orders".into(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                    columns: vec![saya_types::Column {
                        name: "id".into(),
                        data_type: "bigint".into(),
                        nullable: false,
                    }],
                }],
            }],
        }],
    };
    store.upsert_schema(profile.as_str(), &live).await.unwrap();
    let obj = object_ref(&profile, "orders");
    let pending = put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let gone_obj = object_ref(&profile, "shipments");
    let gone = put_item(
        &store,
        &gone_obj,
        ClaimPayload::table_alias("shipments").unwrap(),
        KnowledgeState::Pending,
    )
    .await;

    let (code, out, err) = run(&store, true).await;

    assert_eq!(code, 0, "a partial batch is a success; stderr: {err}");
    // The preview named the set first...
    assert!(
        out.contains("catalog.public.orders"),
        "preview shown: {out}"
    );
    assert!(out.contains("shipments"), "preview shows both: {out}");
    //...the approval was reported per item...
    assert!(
        out.contains(&format!("confirmed {}", pending.as_str())),
        "approval line: {out}"
    );
    //...the refusal was reported per item, with the reason...
    assert!(
        out.contains(&format!("refused {}", gone.as_str())),
        "refusal line names the item: {out}"
    );
    assert!(
        out.contains("no longer in the schema"),
        "refusal line names why: {out}"
    );
    //...and the summary carries both counts.
    assert!(out.contains("approved 1, refused 1"), "summary: {out}");
    // The approval persisted despite the refusal.
    let state = store
        .get_knowledge_item(pending.as_str())
        .await
        .unwrap()
        .expect("item present")
        .state;
    assert_eq!(state, KnowledgeState::Active);
    // Close the store before removing its directory: Windows refuses to
    // delete a directory that still holds an open file, and the SQLite
    // connection lives until the store is dropped.
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}
