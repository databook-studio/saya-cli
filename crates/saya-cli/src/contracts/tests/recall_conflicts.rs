#[tokio::test]
async fn privacy_gate_returns_zero_and_counts_excluded() {
    let root = temp_root("privacy");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["orders".to_string()],
            false,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        0,
        "privacy gate returned contracts"
    );
    assert!(
        outcome.diagnostics.excluded_by_privacy > 0,
        "excluded_by_privacy should be non-zero when matches were suppressed"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 6: an unopenable store reports unavailable without erroring
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unopenable_store_reports_unavailable_without_erroring() {
    let root = temp_root("unavailable");
    // A path whose parent is a regular file cannot be created as a directory.
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);

    let p = profile_a();
    let schema = SchemaTree::default();
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
    assert!(outcome.diagnostics.store_unavailable);
    assert_eq!(outcome.contracts.len(), 0);
    assert_eq!(outcome.diagnostics.considered, 0);
    assert_eq!(outcome.diagnostics.selected, 0);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 8: two TableGrain claims conflict but both are returned
// ---------------------------------------------------------------------------
//
// D-3 NOTE: `table_grain` is a single-valued slot, so two confirmed grains
// cannot coexist in `knowledge_items` — the second `put` replaces the first.
// The conflict's structural precondition is gone under D-3 (see
// `knowledge_validity.rs`). This test now exercises `conflicts_for` directly
// on two constructed claims, so it still proves the detector — one
// `table_grain` conflict naming both ids — without requiring the impossible
// store state. The store-path seeding was the example; the behaviour it
// guarded is the detection.
#[test]
fn two_table_grain_claims_conflict_but_both_returned() {
    use crate::contracts::ContractClaim;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let id1 = ClaimId::parse("c-grain0001").unwrap();
    let id2 = ClaimId::parse("c-grain0002").unwrap();
    let claims = vec![
        ContractClaim {
            id: id1.clone(),
            object: obj.clone(),
            value: ClaimPayload::table_grain("one row per order", None).unwrap(),
            source: ClaimOrigin::UserExplicit,
            status: ClaimStatus::Confirmed,
        },
        ContractClaim {
            id: id2.clone(),
            object: obj.clone(),
            value: ClaimPayload::table_grain("one row per order line", None).unwrap(),
            source: ClaimOrigin::UserExplicit,
            status: ClaimStatus::Confirmed,
        },
    ];
    let conflicts = conflicts_for(&claims);
    let grain_conflicts: Vec<&ContractConflict> = conflicts
        .iter()
        .filter(|c| c.kind == "table_grain")
        .collect();
    assert_eq!(
        grain_conflicts.len(),
        1,
        "expected one table_grain conflict"
    );
    let ids: Vec<ClaimId> = grain_conflicts[0].claim_ids.clone();
    assert!(
        ids.contains(&id1) && ids.contains(&id2),
        "conflict must name both IDs"
    );
    assert_eq!(claims.len(), 2);
}

/// SPEC REVIEW companion: two TableDescription claims do NOT conflict.
///
/// `table_description` is a multi-valued slot under D-3, so two confirmed
/// descriptions coexist in `knowledge_items`; `conflicts_for` does not treat
/// `table_description` as exclusive, so neither is flagged. Seeded through the
/// D-3 store the recall path reads.
#[tokio::test]
async fn two_table_description_claims_do_not_conflict() {
    let root = temp_root("desc_no_conflict");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("sales fact table").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("updated nightly").unwrap(),
        KnowledgeState::Active,
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
    let contract = &outcome.contracts[0];
    assert!(
        contract
            .conflicts
            .iter()
            .all(|c| c.kind != "table_description"),
        "TableDescription must not be flagged as a conflict (SPEC REVIEW)"
    );
    assert_eq!(contract.claims.len(), 2);

    let _ = fs::remove_dir_all(root);
}
