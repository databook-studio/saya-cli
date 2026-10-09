#[tokio::test]
async fn recall_diagnostics_carry_no_claim_text() {
    let root = temp_root("no_text_in_diag");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    const SENTINEL: &str = "SENTINELDIAGTEXT";
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description(format!("orders {SENTINEL} facts")).unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome: RecallOutcome = recall(
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
    let debug = format!("{:?}", outcome.diagnostics);
    assert!(
        !debug.contains(SENTINEL),
        "diagnostics leaked claim text: {debug}"
    );
    let diags: RecallDiagnostics = outcome.diagnostics.clone();
    assert!(!format!("{diags:?}").contains(SENTINEL));

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 10: a forgotten claim disappears from recall immediately
// ---------------------------------------------------------------------------
//
// Under D-3, "forgotten" is `KnowledgeState::Dismissed` on the knowledge item;
// admissibility excludes `Dismissed`, so the item no longer reaches recall.
#[tokio::test]
async fn forgotten_claim_disappears_from_recall() {
    use saya_store::KnowledgeItemStore;
    let root = temp_root("forget_disappears");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("keep").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("gone").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    // The store-assigned `ki-` ids, matched by alias value.
    let items = store
        .knowledge_for_object(&obj)
        .await
        .expect("knowledge items listed");
    let find_id = |alias: &str| {
        items
            .iter()
            .find(|i| {
                matches!(
                    &i.value,
                    ClaimPayload::TableAlias { alias: a, .. } if a == alias
                )
            })
            .map(|i| i.id.clone())
            .expect("alias item stored")
    };
    let keep = ClaimId::parse(&find_id("keep")).unwrap();
    let gone_id = find_id("gone");
    store
        .update_knowledge_item_state(&gone_id, KnowledgeState::Dismissed)
        .await
        .expect("item dismissed");

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
    let contract = &outcome.contracts[0];
    let ids: Vec<ClaimId> = contract.claims.iter().map(|c| c.id.clone()).collect();
    assert!(ids.contains(&keep), "kept claim should remain");
    assert!(
        !ids.iter().any(|id| id.as_str() == gone_id),
        "forgotten claim should not appear"
    );

    let _ = fs::remove_dir_all(root);
}
