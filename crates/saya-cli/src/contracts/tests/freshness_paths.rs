/// A confirmed `default_time_column` item whose binding (`Column{column, Time}`)
/// the matching `table` satisfies, so a fresh cache reads `Current`. Only the
/// cache's *age* varies between the two paths.
async fn seed_current_time_column(
    store: &SqliteStateStore,
    obj: &DatabaseObjectRef,
    _table: &Table,
    column: &str,
) {
    put_item(
        store,
        obj,
        ClaimPayload::default_time_column(column, None).unwrap(),
        KnowledgeState::Active,
    )
    .await;
}

#[tokio::test]
async fn model_path_treats_a_stale_by_age_cache_as_live_schema_unavailable() {
    let root = temp_root("freshness_model_stale_by_age");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let table = table(&[("id", "bigint", false), ("created_at", "timestamp", false)]);
    seed_current_time_column(&store, &obj, &table, "created_at").await;

    // The cache matches the claim, but was observed 25h ago — past the 24h
    // bound. `now` is 25h after observation (0).
    let stale_by_age = SchemaAvailability::available(schema_tree_for(&[("orders", table)]), 0);
    let now = 25 * 60 * 60 * 1000;
    let outcome = recall(
        &store,
        recall_request_with_freshness(
            std::slice::from_ref(&p),
            &[(p.clone(), stale_by_age)],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
            now,
            RetrievalPolicy::ForModel,
        ),
    )
    .await;
    // The contract is NOT dropped (only computed-`Stale` is dropped): it
    // survives, classified `LiveSchemaUnavailable` so the model sees the claim
    // labelled, never silently `Current`.
    assert_eq!(
        outcome.contracts.len(),
        1,
        "a stale-by-age contract is kept for the model, labelled — not dropped"
    );
    assert_eq!(
        outcome.contracts[0].schema_state,
        ContractSchemaState::LiveSchemaUnavailable,
        "a stale-by-age cache cannot vouch for currency on the model path"
    );
    assert_eq!(
        outcome.diagnostics.excluded_by_schema, 0,
        "a stale-by-age contract is not excluded as stale: {:?}",
        outcome.diagnostics
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn human_path_uses_a_stale_by_age_cache_to_classify_current() {
    let root = temp_root("freshness_human_stale_by_age");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let table = table(&[("id", "bigint", false), ("created_at", "timestamp", false)]);
    seed_current_time_column(&store, &obj, &table, "created_at").await;

    // Same 25h-old cache, but the human-review path is unbounded: the cache
    // classifies normally. `contracts list`/`show`/`queue` rely on this — a
    // reviewer looking at their own contracts is not asked to trust a query
    // built on them.
    let stale_by_age = SchemaAvailability::available(schema_tree_for(&[("orders", table)]), 0);
    let now = 25 * 60 * 60 * 1000;
    let outcome = recall(
        &store,
        recall_request_with_freshness(
            std::slice::from_ref(&p),
            &[(p.clone(), stale_by_age)],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
            now,
            RetrievalPolicy::ForHumanReview,
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        1,
        "the human path shows the contract despite the stale-by-age cache"
    );
    assert_eq!(
        outcome.contracts[0].schema_state,
        ContractSchemaState::Current,
        "the human path classifies against the stale-by-age cache as current"
    );

    let _ = fs::remove_dir_all(root);
}
