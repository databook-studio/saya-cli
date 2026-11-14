/// a mixed batch — one `Pending` that confirms and one
/// `Dismissed` that is refused as `Conflict` — asserts the confirmation
/// persisted AND the refusal was reported per-item. An all-success batch
/// proves nothing about the reporting or the partial-success invariants.
#[tokio::test]
async fn approve_all_reports_a_mixed_batch_and_persists_the_successes() {
    let root = temp_root("approve_mixed");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");

    // One candidate still waiting when the sweep reaches it...
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let live_id = item_id_for(&store, &obj, &KnowledgeSlot::TableAlias).await;
    //...and one that was Pending when the queue was read but was dismissed
    // before its turn (another session rejected it). The batch must go through
    // `confirm()` for it and surface the refusal, not hide it.
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("old note").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let gone_id = item_id_for(&store, &obj, &KnowledgeSlot::TableDescription).await;
    store
        .update_knowledge_item_state(gone_id.as_str(), KnowledgeState::Dismissed)
        .await
        .expect("item dismissed");

    let outcomes = approve_all(&store, &[live_id.clone(), gone_id.clone()]).await;

    assert_eq!(outcomes.len(), 2, "every queued item reports an outcome");

    // The survivor confirmed — and the confirmation persisted.
    let (_, live_result) = outcomes
        .iter()
        .find(|(id, _)| id == &live_id)
        .expect("live item reported");
    let claim = live_result.as_ref().expect("pending candidate confirmed");
    assert_eq!(claim.status, ClaimStatus::Confirmed);
    assert_eq!(
        item_state(&store, &live_id).await,
        KnowledgeState::Active,
        "the confirmation persisted despite the sibling refusal"
    );

    // The dismissed item refused as Conflict — reported per-item, not
    // aggregated away, and left Dismissed (not revived).
    let (_, gone_result) = outcomes
        .iter()
        .find(|(id, _)| id == &gone_id)
        .expect("dismissed item reported");
    assert_eq!(
        gone_result.as_ref().unwrap_err(),
        &ContractOpError::Conflict,
        "a dismissed item is refused and the refusal is reported"
    );
    assert_eq!(
        item_state(&store, &gone_id).await,
        KnowledgeState::Dismissed,
        "the refusal changed nothing"
    );

    let _ = fs::remove_dir_all(root);
}

/// an empty queue is a clean no-op — an empty outcome, not
/// an error — and nothing outside the named set is touched (scope is explicit).
#[tokio::test]
async fn approve_all_on_an_empty_queue_is_a_clean_noop() {
    let root = temp_root("approve_empty");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    // A candidate that is NOT in the set — it must be untouched afterwards.
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let untouched = item_id_for(&store, &obj, &KnowledgeSlot::TableAlias).await;

    let outcomes = approve_all(&store, &[]).await;

    assert!(
        outcomes.is_empty(),
        "an empty queue is a clean no-op, not an error"
    );
    assert_eq!(
        item_state(&store, &untouched).await,
        KnowledgeState::Pending,
        "nothing outside the named set is touched"
    );

    let _ = fs::remove_dir_all(root);
}
