use super::*;

// Test (needs_review): a needs_review claim still reaches the model, labelled.
// ---------------------------------------------------------------------------
//
// The distinction that must survive: only computed `Stale` is excluded.
// `NeedsReview` still reaches the model — if it were excluded too, a single
// unrelated column added to a wide table would silently mute every claim on
// that table, and `Stale` and `NeedsReview` would stop meaning different
// things. Here the fingerprint moves (an unrelated column was added) but the
// claim's own referenced column is untouched, so validity reads
// `needs_review`, and the block still carries the claim.

// ---------------------------------------------------------------------------
// Test (D-4): an unrelated column change does NOT drop a claim — it reads
// `current`, and the block still carries the claim.
// ---------------------------------------------------------------------------
//
// Under the old whole-table fingerprint model this scenario read `needs_review`
// (the fingerprint moved, the claim's own column was untouched). Under D-4 a
// fact depends only on what its `SchemaBinding` names — a `default_time_column`
// on `created_at` depends on `created_at` existing with a temporal type, and
// adding an unrelated `note` column does not touch that. So the claim reads
// `current` (not `needs_review`), and still reaches the model. This is D-4's
// whole point: stop crying wolf on unrelated drift. The "still reaches the
// model" half the old test guarded is strengthened — the claim is not merely
// kept-labelled, it is current. The `needs_review` verdict itself still exists
// and still reaches the model; `needs_review_from_a_non_current_fingerprint`
// below exercises that path (a version mismatch).

#[tokio::test]
async fn needs_review_claim_still_reaches_the_model_labelled() {
    let root = temp_root("needs_review_reaches");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let obj = object(&identity, "orders");

    // The item is filed under a `default_time` binding on `created_at`, then
    // the live schema adds an *unrelated* column (`note`). `created_at` is
    // still present and temporal, so the binding is satisfied and the item
    // reads `current` — the unrelated column does not invalidate under D-4.
    put_item(
        &store,
        &obj,
        ClaimPayload::default_time_column("created_at", None).unwrap(),
        KnowledgeState::Active,
    )
    .await;
    let tree = schema_tree_with(
        &identity,
        "orders",
        &[
            ("id", "bigint", false),
            ("created_at", "timestamp", false),
            ("note", "text", true),
        ],
    );
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();

    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "orders",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(
        blocks.len(),
        1,
        "the claim still reaches the model after unrelated drift"
    );
    let body = &blocks[0].body;
    assert!(
        body.contains("created_at"),
        "the claim is still named: {body}"
    );
    assert!(
        body.contains("current"),
        "under D-4 an unrelated column change reads current, not needs_review: {body}"
    );
    assert!(
        !body.contains("possibly out of date"),
        "an unrelated column change is not staleness: {body}"
    );
    let _ = fs::remove_dir_all(root);
}

/// The `needs_review` verdict still reaches the model, labelled in-band, when
/// it arises under D-4 — which is a fingerprint version this build did not
/// write. The item's binding is fine, but it was derived under a format this
/// build cannot faithfully compare, so it is held for review rather than
/// trusted or dropped.
#[tokio::test]
async fn needs_review_from_a_non_current_fingerprint_still_reaches_the_model_labelled() {
    use saya_types::{ColumnRequirement, SchemaBinding};
    let root = temp_root("needs_review_version");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let obj = object(&identity, "orders");
    let tree = schema_tree_with(&identity, "orders", &[("created_at", "timestamp", false)]);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    // A binding the ingest path would derive, but written under a future
    // fingerprint version this build does not know.
    let binding = serde_json::to_string(&SchemaBinding::Column {
        column: "created_at".to_string(),
        requirement: ColumnRequirement::Time,
    })
    .unwrap();
    put_item_with_binding(
        &store,
        &obj,
        ClaimPayload::default_time_column("created_at", None).unwrap(),
        KnowledgeState::Active,
        binding,
        saya_types::FINGERPRINT_VERSION + 1,
    )
    .await;

    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "orders",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(
        blocks.len(),
        1,
        "a needs_review item still reaches the model"
    );
    let body = &blocks[0].body;
    assert!(
        body.contains("created_at"),
        "the item is still named: {body}"
    );
    assert!(
        body.contains("needs_review"),
        "needs_review state is shown in-band: {body}"
    );
    let _ = fs::remove_dir_all(root);
}
