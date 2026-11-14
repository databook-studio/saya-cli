/// With no cached schema the mapping fetches the live schema and the report
/// names `live` — the two schema sources stay distinguishable. The empty test
/// database discovers no tables, so the items are unavailable and nothing is
/// written.
#[tokio::test]
async fn import_without_a_cached_schema_fetches_live_and_names_the_source() {
    let root = temp_root("live_fetch");
    let runtime = runtime_at(&root);
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    // Migrations run on first use; the schema cache stays untouched (missing).

    let items = vec![context_item(
        "analytics",
        "public",
        "orders",
        &ClaimPayload::table_description("One row per confirmed order.").unwrap(),
    )];
    let path = root.join("context.json");
    write_context_file(&path, &context_document(items));

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
    assert_ne!(code, 0, "nothing landed: {out}{err}");
    assert!(
        out.contains("schema: live"),
        "the live fetch is named: {out}"
    );
    assert!(out.contains("unavailable 1"), "{out}");
    assert!(
        stored_items(&store, &runtime, None, "analytics.public.orders")
            .await
            .is_empty(),
        "an import against an empty live schema writes nothing"
    );

    drop(store);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn imported_confirmation_is_not_authority() {
    let source_root = temp_root("authority_source");
    let source_runtime = runtime_at(&source_root);
    let source_store = store_at(&source_root, &source_runtime).await;
    remember_active(
        &source_store,
        &source_runtime,
        ClaimKindArg::Description,
        "One row per confirmed order.",
        None,
    )
    .await;
    let export_path = source_root.join("context.json");
    let (code, out, err) = run(
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
    assert_eq!(code, 0, "export failed: {out}{err}");

    // A second, independent root imports the file. Its schema cache names the
    // same table so the objects resolve.
    let target_root = temp_root("authority_target");
    let target_runtime = runtime_at(&target_root);
    let target_store = store_at(&target_root, &target_runtime).await;
    let identity = identity_for(&target_runtime, "local");
    target_store
        .upsert_schema(&identity, &orders_schema(&["orders"]))
        .await
        .unwrap();

    let (code, out, err) = run(
        ContractsCommand::Import {
            path: export_path.clone(),
            profile: None,
            preview: false,
        },
        &target_runtime,
        &target_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "import failed: {out}{err}");
    assert!(out.contains("imported 1"), "summary: {out}");
    assert!(
        out.contains("Imported items are pending review: saya contracts queue"),
        "the queue pointer: {out}"
    );
    // The opaque identity of neither root may appear.
    let source_identity = identity_for(&source_runtime, "local");
    assert!(
        !out.contains(&source_identity) && !out.contains(&identity),
        "opaque identity leaked into import output: {out}"
    );

    // The written item is Pending under the TARGET profile's identity — an
    // imported confirmation is never an Active one.
    let items = stored_items(
        &target_store,
        &target_runtime,
        None,
        "analytics.public.orders",
    )
    .await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0].state, KnowledgeState::Pending, "{items:?}");
    assert_eq!(items[0].source, ClaimOrigin::TeamFile, "{items:?}");

    // Recall reads Active claims only, so `contracts list` must show nothing
    // for the imported item.
    let (code, out, err) = run(
        ContractsCommand::List { profile: None },
        &target_runtime,
        &target_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "list failed: {out}{err}");
    assert!(
        !out.contains("One row per confirmed order"),
        "a pending import must not recall as confirmed: {out}"
    );

    drop(target_store);
    drop(source_store);
    let _ = fs::remove_dir_all(source_root);
    let _ = fs::remove_dir_all(target_root);
}
