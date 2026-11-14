#[tokio::test]
async fn cross_profile_alias_resolves_only_in_its_own_profile() {
    let root = temp_root("cross_profile");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let a = profile_a();
    let b = profile_b();
    let obj_a = object_ref(&a, "orders");
    let obj_b = object_ref(&b, "orders");

    put_item(
        &store,
        &obj_a,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        &store,
        &obj_b,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema_a = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&a),
            &[(a.clone(), avail(schema_a))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    assert_eq!(outcome.contracts[0].object, obj_a);

    let schema_b = schema_tree_for(&[("orders", table(&[("id", "int", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&b),
            &[(b.clone(), avail(schema_b))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    assert_eq!(outcome.contracts[0].object, obj_b);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 2: candidate claims never appear in recall
// ---------------------------------------------------------------------------
#[tokio::test]
async fn candidate_claims_never_appear_in_recall() {
    let root = temp_root("no_candidates");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");

    // An active alias plus a pending "secret_alias" on the same object. Under
    // `Confirmed` the pending item is not admitted, so it must not reach the
    // recalled contract.
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("secret_alias").unwrap(),
        KnowledgeState::Pending,
    )
    .await;

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    let claims = &outcome.contracts[0].claims;
    assert!(claims.iter().all(|c| c.status.is_recallable()));
    assert!(
        !claims.iter().any(|c| {
            matches!(
                &c.value,
                ClaimPayload::TableAlias { alias, .. } if alias == "secret_alias"
            )
        }),
        "candidate alias leaked into recall"
    );

    let _ = fs::remove_dir_all(root);
}
