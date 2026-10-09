#[tokio::test]
async fn context_profile_mapping_cannot_cross_connections() {
    let root = temp_root("cross_connections");
    let runtime = runtime_with(&root, &["other"]);
    let store = store_at(&root, &runtime).await;
    // `other` caches a schema naming `orders`; `local` does not.
    let other_identity = identity_for(&runtime, "other");
    store
        .upsert_schema(&other_identity, &orders_schema(&["orders"]))
        .await
        .unwrap();

    let items = vec![context_item(
        "analytics",
        "public",
        "orders",
        &ClaimPayload::table_description("One row per confirmed order.").unwrap(),
    )];
    let path = root.join("context.json");
    write_context_file(&path, &context_document(items));

    // Import against `other`: the object resolves there, and the item must be
    // keyed by `other`'s identity.
    let (code, out, err) = run(
        ContractsCommand::Import {
            path: path.clone(),
            profile: Some("other".into()),
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "import onto `other` failed: {out}{err}");
    assert!(out.contains("imported 1"), "{out}");
    assert!(
        out.contains("schema: cached"),
        "the schema source is stated: {out}"
    );
    let items = stored_items(&store, &runtime, Some("other"), "analytics.public.orders").await;
    assert_eq!(
        items.len(),
        1,
        "written under the chosen profile: {items:?}"
    );
    assert!(
        stored_items(&store, &runtime, Some("local"), "analytics.public.orders")
            .await
            .is_empty(),
        "nothing may leak onto the other profile"
    );

    // The same file against `local` — whose schema does not name `orders` —
    // resolves nothing: every item is reported unavailable and nothing lands.
    let (code, out, err) = run(
        ContractsCommand::Import {
            path: path.clone(),
            profile: None,
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        code, 0,
        "an import that lands nothing must exit non-zero: {out}{err}"
    );
    assert!(out.contains("unavailable 1"), "summary: {out}");
    assert!(
        out.contains("orders"),
        "the unavailable item is named: {out}"
    );
    assert!(
        stored_items(&store, &runtime, Some("local"), "analytics.public.orders")
            .await
            .is_empty(),
        "an unavailable item is never written"
    );

    drop(store);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn conflicting_time_columns_require_decision() {
    let source_root = temp_root("conflict_source");
    let source_runtime = runtime_at(&source_root);
    let source_store = store_at(&source_root, &source_runtime).await;
    remember_active(
        &source_store,
        &source_runtime,
        ClaimKindArg::TimeColumn,
        "order_date",
        None,
    )
    .await;
    let export_path = source_root.join("context.json");
    let (code, _out, err) = run(
        ContractsCommand::Export {
            path: export_path.clone(),
            profile: None,
            overwrite: false,
        },
        &source_runtime,
        &source_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "export failed: {err}");

    // The target profile holds a DIFFERENT active default time column for the
    // same object. Both claims cannot hold: the local decision stands, the
    // import is reported as a conflict, and nothing is written.
    let target_root = temp_root("conflict_target");
    let target_runtime = runtime_at(&target_root);
    let target_store = store_at(&target_root, &target_runtime).await;
    let identity = identity_for(&target_runtime, "local");
    target_store
        .upsert_schema(&identity, &orders_schema(&["orders"]))
        .await
        .unwrap();
    remember_active(
        &target_store,
        &target_runtime,
        ClaimKindArg::TimeColumn,
        "return_date",
        None,
    )
    .await;

    let (code, out, err) = run(
        ContractsCommand::Import {
            path: export_path,
            profile: None,
            preview: false,
        },
        &target_runtime,
        &target_store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        code, 0,
        "nothing landed, so the import exits non-zero: {out}{err}"
    );
    assert!(out.contains("conflicts 1"), "summary: {out}");
    assert!(out.contains("conflict"), "the conflict is reported: {out}");
    let items = stored_items(
        &target_store,
        &target_runtime,
        None,
        "analytics.public.orders",
    )
    .await;
    assert_eq!(items.len(), 1, "the import wrote nothing: {items:?}");
    assert_eq!(items[0].state, KnowledgeState::Active);
    assert!(
        matches!(
            &items[0].value,
            ClaimPayload::DefaultTimeColumn { column, .. } if column == "return_date"
        ),
        "the local claim stands: {:?}",
        items[0].value
    );
    assert!(
        out.contains(&items[0].id),
        "the conflict names the existing claim id: {out}"
    );

    drop(target_store);
    drop(source_store);
    let _ = fs::remove_dir_all(source_root);
    let _ = fs::remove_dir_all(target_root);
}
