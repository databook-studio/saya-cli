/// Without `--yes` the queue is printed and nothing is approved — the
/// deny-by-default consent path, including under `--non-interactive`.
#[tokio::test]
async fn without_yes_the_queue_is_printed_and_nothing_is_approved() {
    let root = std::env::temp_dir().join(format!(
        "saya-approve-preview-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = store_at(&root).await;
    let profile = identity();
    let obj = object_ref(&profile, "orders");
    let pending = put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Pending,
    )
    .await;

    let (code, out, _err) = run(&store, false).await;

    assert_eq!(code, EXIT_CONTRACT_ERROR, "refusal exits the contract code");
    assert!(
        out.contains("catalog.public.orders"),
        "preview shown: {out}"
    );
    assert!(
        out.contains("without --yes"),
        "the refusal says how to proceed: {out}"
    );
    assert!(!out.contains("confirmed "), "nothing was approved: {out}");
    let state = store
        .get_knowledge_item(pending.as_str())
        .await
        .unwrap()
        .expect("item present")
        .state;
    assert_eq!(state, KnowledgeState::Pending, "nothing was written");
    // Close the store before removing its directory: Windows refuses to
    // delete a directory that still holds an open file, and the SQLite
    // connection lives until the store is dropped.
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}
