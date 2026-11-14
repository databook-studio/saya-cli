#[tokio::test]
async fn context_import_is_idempotent_and_atomic() {
    let root = temp_root("idempotent");
    let runtime = runtime_at(&root);
    let store = store_at(&root, &runtime).await;
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema(&["orders"]))
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
    ];
    let path = root.join("context.json");
    write_context_file(&path, &context_document(items));

    let (code, out, _err) = run(
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
    assert_eq!(code, 0, "first import: {out}");
    assert!(out.contains("imported 2"), "{out}");
    let first = stored_items(&store, &runtime, None, "analytics.public.orders").await;
    assert_eq!(first.len(), 2);

    // Re-importing the same file is the no-op: identical items are skipped,
    // and the store holds exactly the same rows.
    let (code, out, _err) = run(
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
    assert_eq!(code, 0, "re-import: {out}");
    assert!(out.contains("skipped 2"), "identical items skip: {out}");
    assert_eq!(
        stored_items(&store, &runtime, None, "analytics.public.orders")
            .await
            .len(),
        2,
        "a skipped import writes nothing"
    );

    // Atomicity: a document that validates but carries one item the store's
    // admission gate refuses (an absolute path is machine state, not business
    // context) must write NOTHING — not even its valid sibling.
    let poisoned = vec![
        context_item(
            "analytics",
            "public",
            "orders",
            &ClaimPayload::table_description("See the notes at /home/alice/notes.txt.").unwrap(),
        ),
        context_item(
            "analytics",
            "public",
            "orders",
            &ClaimPayload::table_alias("never-lands").unwrap(),
        ),
    ];
    let poisoned_path = root.join("poisoned.json");
    write_context_file(&poisoned_path, &context_document(poisoned));
    let (code, out, err) = run(
        ContractsCommand::Import {
            path: poisoned_path,
            profile: None,
            preview: false,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(code, 0, "a refused batch exits non-zero: {out}{err}");
    let after = stored_items(&store, &runtime, None, "analytics.public.orders").await;
    assert_eq!(after.len(), 2, "the whole batch rolled back: {after:?}");
    assert!(
        !out.contains("never-lands") || out.contains("refused"),
        "no partial success may be reported: {out}"
    );

    drop(store);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn memory_off_import_does_not_silently_enable_recall() {
    let root = temp_root("memory_off");
    // No [memory] section: the resolved mode is the default `off`.
    let runtime = runtime_at(&root);
    assert_eq!(runtime.resolved.memory.mode, MemoryMode::Off);
    let store = store_at(&root, &runtime).await;
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema(&["orders"]))
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
    assert_eq!(code, 0, "import with memory off still works: {out}{err}");
    assert!(out.contains("imported 1"), "{out}");
    assert!(
        out.contains("Memory is off, so these claims are not used until you enable it."),
        "the memory-off note: {out}"
    );
    // The claims landed pending, and recall stays off: nothing recalled.
    let items = stored_items(&store, &runtime, None, "analytics.public.orders").await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].state, KnowledgeState::Pending);

    drop(store);
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// dbt manifest import + preview
// ---------------------------------------------------------------------------
