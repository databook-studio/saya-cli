/// One credential-shaped item among clean ones is skipped with a reason and
/// never reaches the store's batch; the clean items still land. The preview
/// reports the same classification without writing anything.
#[tokio::test]
async fn credential_shaped_item_is_skipped_and_the_rest_still_imports() {
    let root = temp_root("credential_skip");
    let runtime = runtime_at(&root);
    let store = store_at(&root, &runtime).await;
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema(&["orders", "customers"]))
        .await
        .unwrap();

    let items = vec![
        context_item(
            "analytics",
            "public",
            "orders",
            &ClaimPayload::table_description("One row per confirmed order.").unwrap(),
        ),
        context_item(
            "analytics",
            "public",
            "orders",
            &ClaimPayload::table_alias("customers").unwrap(),
        ),
        context_item(
            "analytics",
            "public",
            "customers",
            &ClaimPayload::table_description("The ingest box runs with password=hunter2.").unwrap(),
        ),
    ];
    let path = root.join("context.json");
    write_context_file(&path, &context_document(items));

    // Preview first: the classification is the same shape the import reports.
    let (code, out, err) = run(
        ContractsCommand::Import {
            path: path.clone(),
            profile: None,
            preview: true,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "preview failed: {out}{err}");
    assert!(out.contains("would import 2"), "preview summary: {out}");
    assert!(
        out.contains("unavailable 1"),
        "preview counts the refusal: {out}"
    );
    assert!(
        out.contains("credential-shaped text"),
        "the refusal names its reason: {out}"
    );
    assert!(
        stored_items(&store, &runtime, None, "analytics.public.orders")
            .await
            .is_empty(),
        "a preview writes nothing"
    );

    // The real import lands the two clean items and reports the refused one.
    let (code, out, err) = run(
        ContractsCommand::Import {
            path,
            profile: None,
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "the clean items still land: {out}{err}");
    assert!(out.contains("imported 2"), "summary: {out}");
    assert!(out.contains("unavailable 1"), "summary: {out}");
    assert!(
        out.contains("credential-shaped text"),
        "the refusal names its reason: {out}"
    );
    let orders = stored_items(&store, &runtime, None, "analytics.public.orders").await;
    assert_eq!(orders.len(), 2, "the clean items are on file: {orders:?}");
    assert!(
        orders
            .iter()
            .all(|item| item.state == KnowledgeState::Pending),
        "landed items are pending: {orders:?}"
    );
    assert!(
        stored_items(&store, &runtime, None, "analytics.public.customers")
            .await
            .is_empty(),
        "the refused item is never written"
    );

    drop(store);
    let _ = fs::remove_dir_all(root);
}

/// The dbt path shares the mapping, so a manifest whose description is
/// credential-shaped skips that item with the same reason while the other
/// models' items still land.
#[tokio::test]
async fn dbt_manifest_credential_shaped_description_is_skipped_and_the_rest_imports() {
    let root = temp_root("dbt_credential_skip");
    let runtime = runtime_at(&root);
    let store = store_at(&root, &runtime).await;
    let identity = identity_for(&runtime, "local");
    store.upsert_schema(&identity, &dbt_schema()).await.unwrap();

    let manifest = root.join("manifest.json");
    fs::write(
        &manifest,
        r#"{
  "metadata": {
    "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json",
    "dbt_version": "1.8.6"
  },
  "nodes": {
    "model.jaffle.orders": {
      "resource_type": "model",
      "name": "orders",
      "alias": "public_orders",
      "database": "analytics",
      "schema": "core",
      "description": "One row per confirmed order."
    },
    "model.jaffle.customers": {
      "resource_type": "model",
      "name": "customers",
      "database": "analytics",
      "schema": "core",
      "description": "The ingest box runs with password=hunter2."
    }
  }
}"#,
    )
    .unwrap();

    let (code, out, err) = run(
        ContractsCommand::ImportDbt {
            manifest,
            profile: None,
            select: vec![],
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "the clean model still lands: {out}{err}");
    assert!(out.contains("imported 1"), "summary: {out}");
    assert!(out.contains("unavailable 1"), "summary: {out}");
    assert!(
        out.contains("credential-shaped text"),
        "the refusal names its reason: {out}"
    );
    let orders = stored_items(&store, &runtime, None, "analytics.core.public_orders").await;
    assert_eq!(orders.len(), 1, "{orders:?}");
    assert_eq!(orders[0].state, KnowledgeState::Pending);
    assert!(
        stored_items(&store, &runtime, None, "analytics.core.customers")
            .await
            .is_empty(),
        "the refused description is never written"
    );

    drop(store);
    let _ = fs::remove_dir_all(root);
}
