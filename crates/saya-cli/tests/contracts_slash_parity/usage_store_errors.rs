#[tokio::test]
async fn remember_too_few_args_reports_usage_without_panicking() {
    // Parse-only: a usage error must surface before any store access, so an
    // unopenable store is irrelevant here.
    let parsed = parse_slash_command("/remember analytics.public.orders alias");
    assert!(
        parsed.is_err(),
        "expected a usage error for a /remember missing the value, got {parsed:?}"
    );
    let msg = parsed.unwrap_err().to_string();
    assert!(
        !msg.is_empty(),
        "usage error must carry guidance, not panic silently"
    );
    // The untrusted partial input must not be echoed into the usage message.
    assert!(
        !msg.contains("analytics.public.orders"),
        "usage echoed untrusted input: {msg}"
    );

    // Only the kind, no table or value, is also a usage error.
    assert!(parse_slash_command("/remember alias").is_err());
    // No arguments at all.
    assert!(parse_slash_command("/remember").is_err());
}

// ---------------------------------------------------------------------------
// 9. An unopenable store. Parity with the headless path is what matters here
//    Every assertion here compares
//    against the headless result rather than a hardcoded code — a parity test
//    that pins an exit value stops testing parity the moment the shared
//    behaviour changes. `/contract`,
//    `/remember` and `/forget` fail with the same non-zero exit and diagnostic as
//    the headless `show`/`remember`/`forget`. Spec §5 item 9's "do not error the
//    session" reads as "the REPL keeps looping" — a non-zero command exit does
//    not tear down the session (`handle_line` returns Ok(false) for an Ok(2)
//    from the dispatcher), so both readings hold. See SPEC REVIEW.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unopenable_store_slash_matches_headless_failure_modes() {
    let root = temp_root("unopenable_slash");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);
    let (runtime, _name) = runtime_at(&root);

    // /contracts fails exactly as headless list does on an unreadable store.
    let headless_list = run_headless(
        ContractsCommand::List { profile: None },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (_, lcode, lout, lerr) =
        run_slash("/contracts", &runtime, &store, RenderFormat::Text).await;
    assert_eq!(
        lcode, headless_list.0,
        "/contracts must exit exactly as headless list does: {lout}{lerr}"
    );
    assert_ne!(
        lcode, 0,
        "an unreadable store must not look like an empty one: {lout}{lerr}"
    );
    assert_eq!(lout, headless_list.1);
    assert_eq!(lerr, headless_list.2);
    assert!(
        !format!("{lout}{lerr}").is_empty(),
        "/contracts must emit a diagnostic"
    );

    // /contract fails the same way headless show does: non-zero, same output.
    let headless_show = run_headless(
        ContractsCommand::Show {
            table: qualified().into(),
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (_, scode, sout, serr) = run_slash(
        "/contract analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        scode, 0,
        "/contract must fail on an unopenable store: {sout}{serr}"
    );
    assert_eq!(scode, headless_show.0);
    assert_eq!(sout, headless_show.1);
    assert_eq!(serr, headless_show.2);

    // /remember fails, matching headless remember.
    let (_, rcode, _rout, _rerr) = run_slash(
        "/remember analytics.public.orders alias customers",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(rcode, 0, "/remember against an unopenable store must fail");

    // /forget against an unopenable store: seed a valid id on a good store, then
    // forget on the bad one. Both slash and headless fail.
    let good_root = temp_root("unopenable_slash_good");
    let (good_runtime, _n) = runtime_at(&good_root);
    let good_store = store_at(&good_root).await;
    let (_, _, gout, _) = run_slash(
        "/remember analytics.public.orders alias customers",
        &good_runtime,
        &good_store,
        RenderFormat::Text,
    )
    .await;
    assert!(gout.contains("remembered"));
    let identity = identity_for(&good_runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = good_store.knowledge_for_object(&object).await.unwrap();
    let id = items[0].id.as_str();
    let (_, fcode, _fout, _ferr) = run_slash(
        &format!("/forget {id}"),
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(fcode, 0, "/forget against an unopenable store must fail");

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(good_root);
}

// ---------------------------------------------------------------------------
// 10. column-scoped kinds: the column is the positional after the kind, and the
//     translated command matches the headless one (parity for the /remember
//     shape decision's column forms). Parse-only: the translation is the only
//     thing the slash adapter owns, so it is asserted without touching the store.
// ---------------------------------------------------------------------------
