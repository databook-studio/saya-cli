use super::*;

// Phase 4b: recall = include-candidates admits a candidate, plainly labelled
// unconfirmed; recall = confirmed keeps today's behaviour (no candidate).
// ---------------------------------------------------------------------------

/// recall = include-candidates: the candidate reaches the block, and the body
/// marks it unconfirmed so the model cannot read it as an established fact
///. The confirmed alias is unmarked.
#[tokio::test]
async fn include_candidates_admits_candidate_plainly_labelled_unconfirmed() {
    let root = temp_root("include_candidates");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    seed_orders_confirmed_and_candidate(&store, &identity).await;

    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::IncludeCandidates,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(blocks.len(), 1, "one block with the candidate admitted");
    let body = &blocks[0].body;
    // The candidate's column reaches the body — it was admitted.
    assert!(
        body.contains("created_at"),
        "candidate claim reaches the body"
    );
    // The candidate is plainly labelled unconfirmed, in-band on its line, so a
    // model cannot read it as an established fact (ADR 0002 §4). The label is a
    // fixed token the render layer owns; asserting the token keeps the marker
    // honest against a future change that softens it.
    assert!(
        body.contains("candidate"),
        "candidate claim is labelled as a candidate: {body}"
    );
    assert!(
        body.contains("unconfirmed") || body.contains("not confirmed"),
        "candidate claim is marked unconfirmed: {body}"
    );
    let _ = fs::remove_dir_all(root);
}

/// recall = confirmed: the candidate is excluded — today's behaviour, unchanged
///. Only the confirmed alias reaches the block, and it
/// carries no candidate marker.
#[tokio::test]
async fn confirmed_excludes_candidates_unchanged_behaviour() {
    let root = temp_root("confirmed_excludes");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    seed_orders_confirmed_and_candidate(&store, &identity).await;

    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(blocks.len(), 1, "confirmed recall still produces a block");
    let body = &blocks[0].body;
    // The confirmed alias reaches the block.
    assert!(body.contains("orders"), "confirmed alias reaches the body");
    // The candidate's column does NOT reach the body under confirmed recall.
    assert!(
        !body.contains("created_at"),
        "candidate claim is excluded under confirmed recall: {body}"
    );
    // And no candidate marker appears, since no candidate was admitted.
    assert!(
        !body.contains("candidate"),
        "no candidate marker when none was admitted: {body}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Bounds from config are honoured: lowering `max_contracts` (max_objects) to 1
/// returns exactly one contract even when two match.
#[tokio::test]
async fn bounds_from_config_lowering_max_contracts_returns_one_contract() {
    let root = temp_root("bounds_one");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    // Two matching objects, each with a confirmed claim.
    let tree = many_orders_schema(&identity, 2);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&orders_table());
    for i in 0..2 {
        let obj = object(&identity, &format!("orders{i}"));
        remember_confirmed_default_time_column(&store, &obj, &fp, "created_at").await;
    }
    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "orders",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds {
            max_objects: 1,
            max_claims_per_object: 12,
            max_bytes: 16384,
        },
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(blocks.len(), 1, "one block");
    // max_objects = 1 truncates: the block flags it, and the body carries one
    // object only.
    assert!(blocks[0].truncated, "lowering max_contracts truncates");
    assert!(
        blocks[0].body.matches("orders0").count() == 1
            || blocks[0].body.matches("orders1").count() == 1,
        "exactly one object's claims in the body"
    );
    let _ = fs::remove_dir_all(root);
}
