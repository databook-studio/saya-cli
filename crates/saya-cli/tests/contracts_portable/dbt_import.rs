#[tokio::test]
async fn import_dbt_fixture_lands_pending_descriptions_and_relationship() {
    let root = temp_root("dbt_v12");
    let runtime = runtime_at(&root);
    let store = store_at(&root, &runtime).await;
    let identity = identity_for(&runtime, "local");
    store.upsert_schema(&identity, &dbt_schema()).await.unwrap();

    let manifest = root.join("manifest.json");
    fs::write(
        &manifest,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/contracts/dbt/fixtures/v12_manifest.json"
        )),
    )
    .unwrap();

    let (code, out, err) = run(
        ContractsCommand::ImportDbt {
            manifest: manifest.clone(),
            profile: None,
            select: vec![],
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "dbt import failed: {out}{err}");
    // The fixture maps 9 items: 6 descriptions and 3 relationships.
    assert!(out.contains("imported 9"), "summary: {out}");
    assert!(
        out.contains("Imported items are pending review: saya contracts queue"),
        "the queue pointer: {out}"
    );

    // Every landed row is Pending with dbt provenance.
    for (name, qualified) in [
        ("public_orders", "analytics.core.public_orders"),
        ("customers", "analytics.core.customers"),
        ("hidden", "analytics.core.hidden"),
        ("raw_events", "raw.jaffle.raw_events"),
    ] {
        let items = stored_items(&store, &runtime, None, qualified).await;
        assert!(!items.is_empty(), "{name} has no imported items");
        assert!(
            items
                .iter()
                .all(|item| item.state == KnowledgeState::Pending),
            "{name} items must be pending: {items:?}"
        );
        assert!(
            items
                .iter()
                .all(|item| item.source == ClaimOrigin::TeamFile),
            "{name} items carry the team-file origin: {items:?}"
        );
    }
    // The relationships landed as join rules — the one relationship-shaped
    // slot the store can file — with the paired columns as the join keys.
    let orders = stored_items(&store, &runtime, None, "analytics.core.public_orders").await;
    let joins: Vec<_> = orders
        .iter()
        .filter(|item| item.slot == KnowledgeSlot::RelationJoinRule)
        .collect();
    assert_eq!(
        joins.len(),
        2,
        "orders carries two imported relationships: {orders:?}"
    );
    assert!(
        joins.iter().any(|item| matches!(
            &item.value,
            ClaimPayload::JoinRule { target, local_columns, target_columns, .. }
                if target == "analytics.core.customers"
                    && local_columns == &["customer_id".to_string()]
                    && target_columns == &["id".to_string()]
        )),
        "the ref relationship filed as a keyed join rule: {joins:?}"
    );
    assert!(
        joins.iter().any(|item| matches!(
            &item.value,
            ClaimPayload::JoinRule { target, local_columns, target_columns, .. }
                if target == "raw.jaffle.raw_events"
                    && local_columns == &["event_id".to_string()]
                    && target_columns == &["event_key".to_string()]
        )),
        "the source relationship filed as a keyed join rule: {joins:?}"
    );

    // Re-importing the same manifest is idempotent.
    let (code, out, _err) = run(
        ContractsCommand::ImportDbt {
            manifest: manifest.clone(),
            profile: None,
            select: vec![],
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "re-import: {out}");
    assert!(out.contains("skipped 9"), "identical items skip: {out}");

    // A --select narrows the mapping: the glob matches node names, so
    // `orders` selects the orders model alone — its three description items —
    // while its relationships' targets (customers, events) are unselected and
    // reported as skipped.
    let fresh_root = temp_root("dbt_select");
    let fresh_runtime = runtime_at(&fresh_root);
    let fresh_store = store_at(&fresh_root, &fresh_runtime).await;
    let fresh_identity = identity_for(&fresh_runtime, "local");
    fresh_store
        .upsert_schema(&fresh_identity, &dbt_schema())
        .await
        .unwrap();
    let (code, out, err) = run(
        ContractsCommand::ImportDbt {
            manifest: manifest.clone(),
            profile: None,
            select: vec!["orders".into()],
            preview: false,
        },
        &fresh_runtime,
        &fresh_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "select import failed: {out}{err}");
    assert!(out.contains("imported 3"), "only orders' own items: {out}");
    assert!(
        out.contains("unavailable 2"),
        "the unselected targets skip: {out}"
    );
    assert!(
        out.contains("unresolved_target"),
        "the skip reason is named: {out}"
    );

    drop(store);
    drop(fresh_store);
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(fresh_root);
}
