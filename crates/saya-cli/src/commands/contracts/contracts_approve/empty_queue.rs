/// An empty queue is a clean no-op: exit 0, one line, nothing approved
/// (at the command layer).
#[tokio::test]
async fn empty_queue_is_a_clean_noop() {
    let root = std::env::temp_dir().join(format!(
        "saya-approve-empty-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = store_at(&root).await;

    let (code, out, _err) = run(&store, true).await;

    assert_eq!(code, 0);
    assert!(
        out.contains("No candidates awaiting approval"),
        "clean no-op message: {out}"
    );
    // Close the store before removing its directory: Windows refuses to
    // delete a directory that still holds an open file, and the SQLite
    // connection lives until the store is dropped.
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}
