#[tokio::test]
async fn remember_unknown_object_against_cached_schema_refuses_and_stores_nothing() {
    let root = temp_root("remember_unknown_object_refuses");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema())
        .await
        .unwrap();

    // `analytics.public.ghost` is well-formed (so it passes parse/validate)
    // but absent from the cached schema — the typo/wrong-scope case.
    let remember = ContractsCommand::Remember {
        table: "analytics.public.ghost".into(),
        kind: ClaimKindArg::Alias,
        value: "nope".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(code, 0, "remember of an unknown object must not succeed");
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("analytics.public.ghost"),
        "error must name the unknown object so the user sees their typo: {combined}"
    );
    assert!(
        combined.contains("connection schema"),
        "error must suggest `connection schema --refresh`: {combined}"
    );
    assert!(
        combined.contains("--refresh"),
        "error must mention the --refresh flag: {combined}"
    );
    // The opaque identity must not reach the error string.
    assert!(
        !combined.contains(&identity),
        "opaque identity leaked into the unknown-object error: {combined}"
    );

    // Nothing was stored for the typo'd object.
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let ghost = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "ghost",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&ghost).await.unwrap();
    assert!(
        items.is_empty(),
        "refuse must store nothing for the unknown object: {items:?}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 15. No-cache guard: with NO cached schema `remember` keeps today's behaviour
//     — it succeeds, and `show` reads `live_schema_unavailable`. Refusing would
//     make remember unusable before a first refresh, so the fix must not touch
//     this path. The binding model does not persist a digest on the row (only
//     the fingerprint *version*), so the "unobserved sentinel" the legacy row
//     carried is replaced by "current version + a `Table` binding + no schema
//     to classify against" — which reads `live_schema_unavailable` honestly.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_with_no_cached_schema_keeps_sentinel_and_succeeds() {
    let root = temp_root("remember_no_cache_sentinel");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // Genuinely no cache for this profile (store_at cached an empty default
    // for its own scope identity; drop it).
    let identity = identity_for(&runtime, "local");
    store.invalidate_schema(&identity).await.unwrap();

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "remember with no cache must still succeed: {err}");
    assert!(out.contains("remembered"), "out: {out}");

    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(items.len(), 1, "{items:?}");
    // The item is stored under the current fingerprint version with a `Table`
    // binding — the no-cache shape that reads `live_schema_unavailable` until a
    // schema is cached. (The digest is not persisted on the row; the version is
    // what `item_validity_for` gates on, alongside the binding.)
    assert_eq!(items[0].fingerprint_version, FINGERPRINT_VERSION);
    assert_eq!(items[0].state, KnowledgeState::Active);

    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("live_schema_unavailable"),
        "no-cache claim must read live_schema_unavailable, not be refused or fabricated current: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

// An EMPTY cached schema (the no-op sentinel `store_at` writes, or a never-
// populated cache) carries no real schema information, so it must be treated
// the same as no cache: `remember` succeeds with the sentinel, it does not
// refuse every object as "unknown".
#[tokio::test]
async fn remember_against_empty_cached_schema_keeps_sentinel_and_succeeds() {
    let root = temp_root("remember_empty_cache_sentinel");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // `store_at` already cached `SchemaTree::default()` (empty databases) for
    // this profile's identity — leave it. An empty tree is "no schema", not
    // "everything is unknown".
    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(
        code, 0,
        "remember against an empty cached schema must succeed, not refuse: {err}"
    );
    assert!(out.contains("remembered"), "out: {out}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 16. ITEM 2 regression: `contracts queue` loads the cached schema (the same
//     helper `list`/`show` use) and reports a real schema state, not a constant
//     `live_schema_unavailable`. Before the fix the queue call site passed
//     `schemas: &[]`, so validity could only ever compute
//     LiveSchemaUnavailable — even right after a `connection schema --refresh`
//     that populated the cache. A candidate whose stored fingerprint equals
//     the cached table's digest must read `current` in the queue, the same as
//     `list`/`show`.
//
//     The claim is seeded as a *candidate* (the queue's subject) carrying the
//     real fingerprint of the cached table — not the unobserved sentinel
//     `remember` stores, which never matches and so could only prove the bug
//     half-fixed.
// ---------------------------------------------------------------------------
async fn seed_current_candidate(store: &SqliteStateStore, runtime: &RuntimeConfig) -> ClaimId {
    let identity = identity_for(runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::table_alias("orders").unwrap();
    seed_pending_item(store, &object, &payload, ClaimOrigin::AssistantInferred).await
}
