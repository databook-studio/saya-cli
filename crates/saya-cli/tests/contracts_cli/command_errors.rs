#[tokio::test]
async fn unknown_profile_is_typed_error_listing_available_names() {
    let root = temp_root("unknown_profile");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let list = ContractsCommand::List {
        profile: Some("nope".into()),
    };
    let (code, out, err) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(code, 0, "unknown profile must not succeed: {out}{err}");
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("nope"),
        "error should name the unknown profile: {combined}"
    );
    assert!(
        combined.contains("local"),
        "error should list available profile names: {combined}"
    );
    // The identity must never reach an error string either.
    let identity = identity_for(&runtime, "local");
    assert!(
        !combined.contains(&identity),
        "opaque identity leaked into error: {combined}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 7. malformed qualified table -> typed error naming the expected form
// ---------------------------------------------------------------------------
#[tokio::test]
async fn malformed_qualified_table_is_typed_error() {
    let root = temp_root("malformed_table");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    for bad in ["orders", "public.orders", "a.b.c.d"] {
        let show = ContractsCommand::Show {
            table: bad.into(),
            profile: None,
        };
        let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
        assert_ne!(code, 0, "{bad:?} must not succeed: {out}{err}");
        let combined = format!("{out}{err}");
        assert!(
            combined.contains("catalog.schema.object"),
            "error should name the expected form for {bad:?}: {combined}"
        );
        assert!(
            !combined.contains(bad),
            "error must not echo untrusted input {bad:?}: {combined}"
        );
    }

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 8. the opaque identity appears in no output (text, JSON, NDJSON)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn opaque_profile_identity_never_reaches_rendered_output() {
    let root = temp_root("identity_leak");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, _out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");

    let identity = identity_for(&runtime, "local");
    assert_eq!(identity.len(), 66);
    assert!(identity.starts_with("p-"));

    let list = ContractsCommand::List { profile: None };
    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    for format in [RenderFormat::Text, RenderFormat::Json, RenderFormat::Ndjson] {
        let (code, list_out, list_err) = run(list.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "list {format:?} stderr: {list_err}");
        let (code, show_out, show_err) = run(show.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "show {format:?} stderr: {show_err}");
        for captured in [list_out, list_err, show_out, show_err] {
            assert!(
                !captured.contains(&identity),
                "opaque identity leaked into {format:?} output: {captured}"
            );
        }
    }

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 9. An unreadable store is not an empty store. `list` exits non-zero so that
//    "you have no contracts" and "I could not read your contracts" stay
//    distinguishable, matching `show`. Writes also fail.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unopenable_store_reads_and_writes_both_exit_nonzero() {
    let root = temp_root("unopenable");
    // A path whose parent is a regular file cannot be created as a directory,
    // so the store pool cannot open.
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);

    let (runtime, _c, _n) = runtime_at(&root);

    let list = ContractsCommand::List { profile: None };
    let (code, out, err) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(
        code, 0,
        "list must exit non-zero on an unopenable store: {out}{err}"
    );
    let combined = format!("{out}{err}");
    assert!(
        !combined.is_empty(),
        "list must emit a diagnostic on an unopenable store"
    );

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, _out, _err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(
        code, 0,
        "a write against an unopenable store must exit non-zero"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 10. JSON and NDJSON carry the same claim ids as the text form
// ---------------------------------------------------------------------------
