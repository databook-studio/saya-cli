#[tokio::test]
async fn malformed_table_rejected_identically_by_slash_and_headless() {
    let root = temp_root("malformed_parity");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let bad = "orders";
    let headless = run_headless(
        ContractsCommand::Show {
            table: bad.into(),
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (_cmd, code, out, err) =
        run_slash("/contract orders", &runtime, &store, RenderFormat::Text).await;

    // Same error class: non-zero exit, same guidance text, no echo of the input.
    assert_ne!(code, 0, "/contract malformed must not succeed: {out}{err}");
    assert_eq!(code, headless.0, "exit code diverged");
    assert_eq!(out, headless.1, "stdout diverged");
    assert_eq!(err, headless.2, "stderr diverged");
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("catalog.schema.object"),
        "missing guidance: {combined}"
    );
    assert!(
        !combined.contains(bad),
        "echoed untrusted input: {combined}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 6. /remember then /contract round-trips the claim (beyond-parity).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_then_contract_round_trips() {
    let root = temp_root("round_trip_slash");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let (_, rcode, rout, rerr) = run_slash(
        "/remember analytics.public.orders alias customers",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(rcode, 0, "stderr: {rerr}");
    assert!(rout.contains("remembered"), "out: {rout}");

    let (_, _, cout, cerr) = run_slash(
        "/contract analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(cerr, "");
    assert!(cout.contains("customers"), "value missing: {cout}");
    assert!(cout.contains("table_alias"), "kind missing: {cout}");
    assert!(cout.contains("user_explicit"), "origin missing: {cout}");
    assert!(cout.contains("confirmed"), "status missing: {cout}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 7. /forget then /contract no longer lists it (beyond-parity).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn forget_then_contract_no_longer_lists() {
    let root = temp_root("forget_then_show_slash");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let (_, _, rout, _) = run_slash(
        "/remember analytics.public.orders alias customers",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert!(rout.contains("remembered"));
    let identity = identity_for(&runtime, "local");
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
    let id = items[0].id.as_str();

    let (_, _, fout, ferr) = run_slash(
        &format!("/forget {id}"),
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(ferr, "");
    assert!(fout.contains("forgotten"), "out: {fout}");

    let (_, _, cout, _) = run_slash(
        "/contract analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert!(!cout.contains("customers"), "still listed: {cout}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 8. /remember with too few arguments reports usage and does not panic.
// ---------------------------------------------------------------------------
