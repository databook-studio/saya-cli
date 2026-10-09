#[tokio::test]
async fn resolve_prefix_matches_a_unique_ki_id_prefix() {
    let root = temp_root("resolve_prefix_unique");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = item_id_for(&store, &obj, &KnowledgeSlot::TableAlias).await;
    // The full id resolves, as does a short unique prefix of it.
    assert_eq!(resolve_prefix(&store, &p, id.as_str()).await.unwrap(), id);
    let short: String = id.as_str().chars().take(6).collect();
    assert_eq!(
        resolve_prefix(&store, &p, &short).await.unwrap(),
        id,
        "a short unique `ki-` prefix resolves to the item"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn resolve_prefix_refuses_ambiguous_and_missing() {
    let root = temp_root("resolve_prefix_refuse");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    // Two Pending items on distinct objects; their `ki-` ids share the `ki-`
    // marker, so a bare "ki-" prefix is ambiguous.
    put_item(
        &store,
        &object_ref(&p, "orders"),
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    put_item(
        &store,
        &object_ref(&p, "returns"),
        ClaimPayload::table_alias("returns").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let err = resolve_prefix(&store, &p, "ki-").await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::Conflict,
        "a prefix matching more than one item is ambiguous, not a guess"
    );
    // A prefix that matches nothing is NotFound, not a panic.
    let err = resolve_prefix(&store, &p, "ki-deadbeef").await.unwrap_err();
    assert_eq!(err, ContractOpError::NotFound);
    // A legacy `c-` prefix matches no `ki-` id — the receipt no longer prints
    // `c-` ids, so a stale `c-` reference refuses rather than silently writing
    // the wrong item.
    let err = resolve_prefix(&store, &p, "c-").await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::NotFound,
        "a legacy `c-` prefix matches no knowledge item"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn review_wrappers_pass_through_and_map_errors() {
    let root = temp_root("review_wrappers");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");

    // A Pending candidate the decision ops act on, seeded on `knowledge_items`
    // (the table confirm/reject/forget now read).
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = item_id_for(&store, &obj, &KnowledgeSlot::TableAlias).await;

    // confirm on an unknown id maps to NotFound. Twiddle the last hex char so
    // the id parses but matches no row.
    let mut chars: Vec<char> = id.as_str().chars().collect();
    if let Some(last) = chars.last_mut() {
        *last = match *last {
            '0'..='8' => char::from_u32(*last as u32 + 1).unwrap(),
            '9' | 'a'..='e' => char::from_u32(*last as u32 + 1).unwrap(),
            'f' => '0',
            _ => '0',
        };
    }
    let fake = ClaimId::parse(&chars.into_iter().collect::<String>()).unwrap();
    let err = confirm(&store, &fake).await.unwrap_err();
    assert_eq!(err, ContractOpError::NotFound);

    // Rejecting the candidate moves it to Dismissed (rendered Rejected), and
    // it no longer renders as a candidate.
    let rejected = reject(&store, &id).await.unwrap();
    assert_eq!(rejected.status, ClaimStatus::Rejected);
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Dismissed);

    // Forgetting a second item on the same object dismisses it; show against a
    // missing schema then finds nothing recallable (every item is Dismissed).
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("the orders table").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let second = item_id_for(&store, &obj, &KnowledgeSlot::TableDescription).await;
    forget(&store, &second, ForgetReason::UserRequest)
        .await
        .unwrap();
    assert_eq!(item_state(&store, &second).await, KnowledgeState::Dismissed);
    let shown = show(
        &store,
        &obj,
        &SchemaAvailability::Missing,
        RetrievalPolicy::ForHumanReview,
        FRESH_NOW,
    )
    .await
    .unwrap();
    assert!(shown.is_none(), "a dismissed-only contract shows None");

    let _ = fs::remove_dir_all(root);
}
