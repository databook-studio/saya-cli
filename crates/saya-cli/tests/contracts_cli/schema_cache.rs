#[tokio::test]
async fn list_and_show_report_current_against_a_cached_schema() {
    let root = temp_root("list_show_cached_schema_state");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    seed_current_claim(&store, &runtime).await;
    // Cache the schema that matches the seeded claim — what
    // `connection schema <profile> --refresh` would have written.
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema())
        .await
        .unwrap();

    // `list` must report `current`, not `live_schema_unavailable`.
    let list = ContractsCommand::List { profile: None };
    let (code, out, err) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "list stderr: {err}");
    assert!(
        out.contains("[current]"),
        "list must report current against the cached schema: {out}"
    );
    assert!(
        !out.contains("live_schema_unavailable"),
        "list must not report live_schema_unavailable when a schema is cached: {out}"
    );

    // `show` must report `current` for the same reason.
    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("[current]"),
        "show must report current against the cached schema: {out}"
    );
    assert!(
        !out.contains("live_schema_unavailable"),
        "show must not report live_schema_unavailable when a schema is cached: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn list_and_show_report_live_schema_unavailable_when_nothing_is_cached() {
    let root = temp_root("list_show_no_cache");
    let (runtime, _c, _n) = runtime_at(&root);
    // A store whose only cached schema is the empty default `store_at` writes
    // for *its* profile — invalidate it so there is genuinely no cache.
    let store = store_at(&root).await;
    let identity = identity_for(&runtime, "local");
    store.invalidate_schema(&identity).await.unwrap();
    seed_current_claim(&store, &runtime).await;

    // With no cached schema, `live_schema_unavailable` is the honest answer —
    // not a fallback the fix fabricates a `current` from.
    let list = ContractsCommand::List { profile: None };
    let (code, out, err) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "list stderr: {err}");
    assert!(
        out.contains("live_schema_unavailable"),
        "list must report live_schema_unavailable when no schema is cached: {out}"
    );

    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("live_schema_unavailable"),
        "show must report live_schema_unavailable when no schema is cached: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 13. Symptom B regression: a claim remembered right AFTER a schema refresh
//     reads `current`, not `needs_review`. Before the fix `remember` stored the
//     all-zero unobserved fingerprint, which can never equal a real digest, so
//     every claim read `needs_review — the schema changed since the claim was
//     made` even though nothing changed. Now `remember` computes the real
//     `of_table` digest against the cached table and stores typed column
//     snapshots, so the claim matches the cache.
// ---------------------------------------------------------------------------
