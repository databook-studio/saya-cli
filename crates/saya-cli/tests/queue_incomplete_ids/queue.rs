//! The queue arm of the A013 regression: a malformed pending row is dropped
//! from the candidates but visibly disclosed as `incomplete`.

use super::*;

#[tokio::test]
async fn queue_marks_a_malformed_pending_id_incomplete_and_keeps_the_valid_candidate() {
    let root = temp_root("malformed-id");
    let runtime = runtime_at(&root);
    let db = root.join("state.sqlite3");
    let store = store_at(&root, &runtime).await;
    let identity = identity_for(&runtime, "local");
    store.invalidate_schema(&identity).await.unwrap();

    let object = object_for(&runtime, "orders");
    seed_pending_alias(&store, &object, "customers").await;
    let valid_id: String = store
        .knowledge_for_object(&object)
        .await
        .expect("knowledge items listed")[0]
        .id
        .clone();
    ClaimId::parse(&valid_id).expect("seeded id parses");
    plant_malformed_pending_id(&store, &db, &object).await;

    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");
    assert!(
        out.contains(&valid_id),
        "the valid candidate must still queue: {out}"
    );
    assert!(
        !out.contains("not a valid id!"),
        "the malformed id must never render: {out}"
    );
    assert!(
        out.contains("incomplete"),
        "the queue must visibly disclose the dropped malformed row: {out}"
    );

    // The malformed sibling is also invisible to JSON: the valid candidate
    // still queues, flagged incomplete.
    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Json).await;
    assert_eq!(code, 0, "queue json stderr: {err}");
    let events: Vec<serde_json::Value> = out
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("line is JSON"))
        .collect();
    assert_eq!(events.len(), 1, "one queue event: {events:?}");
    let items = events[0]["items"]
        .as_array()
        .expect("queue event carries items");
    assert_eq!(items.len(), 1, "only the valid candidate queues: {items:?}");
    assert_eq!(items[0]["claim_id"], valid_id.as_str());
    assert_eq!(items[0]["incomplete"], true);

    let _ = fs::remove_dir_all(root);
}
