#[tokio::test]
async fn queue_lists_candidates_not_confirmed_or_forgotten() {
    let root = temp_root("queue_membership");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let live = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);

    // An Active item (confirmed) and a Dismissed one (rejected/forgotten) must
    // not appear; a Pending candidate must. The queue lists `Pending` only.
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("confirmed_alias").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("cand_alias").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let cand_id = item_id_for(&store, &obj, &KnowledgeSlot::TableAlias).await;
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("forgotten").unwrap(),
        KnowledgeState::Dismissed,
    )
    .await;

    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live))],
        200,
    )
    .await
    .unwrap();

    let ids: Vec<ClaimId> = queued.iter().map(|q| q.claim.id.clone()).collect();
    assert!(ids.contains(&cand_id), "candidate missing from queue");
    // Every queued item is Pending — neither Active nor Dismissed leaks in.
    for id in &ids {
        assert_eq!(
            item_state(&store, id).await,
            KnowledgeState::Pending,
            "non-pending item appeared in the queue"
        );
    }
    // The queue carries one entry per Pending candidate.
    assert_eq!(queued.len(), 1);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_orders_oldest_then_slot_then_id() {
    let root = temp_root("queue_order");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let live = schema_tree_for(&[
        ("old", table(&[("id", "bigint", false)])),
        ("new", table(&[("id", "bigint", false)])),
        ("both", table(&[("id", "bigint", false)])),
    ]);

    // Oldest first: two candidates on distinct objects, created in sequence,
    // must come back oldest-first. A short sleep makes the millisecond stamps
    // differ so the primary key (created_unix_ms) decides.
    put_item(
        &store,
        &object_ref(&p, "old"),
        ClaimPayload::table_alias("old").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    put_item(
        &store,
        &object_ref(&p, "new"),
        ClaimPayload::table_alias("new").unwrap(),
        KnowledgeState::Pending,
    )
    .await;

    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live.clone()))],
        200,
    )
    .await
    .unwrap();
    let ordered: Vec<&str> = queued.iter().map(|q| q.claim.object.object()).collect();
    let old_pos = ordered
        .iter()
        .position(|o| *o == "old")
        .expect("old candidate missing");
    let new_pos = ordered
        .iter()
        .position(|o| *o == "new")
        .expect("new candidate missing");
    assert!(old_pos < new_pos, "oldest-first order broken: {ordered:?}");

    // Slot tie-break: two candidates on the same object, created back-to-back
    // (same ms), fall back to the slot's canonical string. `table.alias` sorts
    // before `table.description`, so the alias comes first.
    let both = object_ref(&p, "both");
    put_item(
        &store,
        &both,
        ClaimPayload::table_alias("both_alias").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    put_item(
        &store,
        &both,
        ClaimPayload::table_description("both description").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live.clone()))],
        200,
    )
    .await
    .unwrap();
    let both_rows: Vec<&str> = queued
        .iter()
        .filter(|q| q.claim.object.object() == "both")
        .map(|q| q.claim.id.as_str())
        .collect();
    let alias_id = item_id_for(&store, &both, &KnowledgeSlot::TableAlias)
        .await
        .as_str()
        .to_string();
    let desc_id = item_id_for(&store, &both, &KnowledgeSlot::TableDescription)
        .await
        .as_str()
        .to_string();
    // `table.alias` < `table.description` lexically, so the alias precedes the
    // description when their created stamps coincide.
    let alias_pos = both_rows
        .iter()
        .position(|id| *id == alias_id)
        .expect("alias in queue");
    let desc_pos = both_rows
        .iter()
        .position(|id| *id == desc_id)
        .expect("description in queue");
    assert!(
        alias_pos < desc_pos,
        "slot tie-break broken (alias should precede description): {both_rows:?}"
    );

    // The full queue order must be identical on a second run — a queue whose
    // order shifts between runs is one a user cannot work through.
    let run_a = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live.clone()))],
        200,
    )
    .await
    .unwrap();
    let run_b = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live))],
        200,
    )
    .await
    .unwrap();
    let ids_a: Vec<ClaimId> = run_a.iter().map(|q| q.claim.id.clone()).collect();
    let ids_b: Vec<ClaimId> = run_b.iter().map(|q| q.claim.id.clone()).collect();
    assert_eq!(ids_a, ids_b, "queue order was not stable across runs");

    let _ = fs::remove_dir_all(root);
}
