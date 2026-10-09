#[tokio::test]
async fn remember_slash_and_headless_produce_same_claim_id() {
    let root = temp_root("remember_parity");
    let (runtime, _name) = runtime_at(&root);

    // Slash form: kind positional, everything after it is the value.
    let slash_store = store_at_name(&root, "slash.sqlite3").await;
    let (slash_cmd, scode, sout, serr) = run_slash(
        "/remember analytics.public.orders alias customers",
        &runtime,
        &slash_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(scode, 0, "/remember stderr: {serr}");
    assert!(
        sout.contains("remembered alias customers for analytics.public.orders"),
        "slash /remember names fact and object: {sout}"
    );
    assert!(
        !sout.contains("ki-"),
        "slash /remember contains no raw id: {sout}"
    );

    // The translated command must be structurally equal to the headless one —
    // same operation, same payload constructors, same deterministic identity.
    assert_eq!(
        slash_cmd,
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::Alias,
            value: "customers".into(),
            column: None,
            reason: None,
            profile: None,
        }
    );

    // A *fresh* store at the same root (same scope) so the headless path stores
    // a new, comparable claim — not a duplicate of the slash claim.
    let headless_store = store_at_name(&root, "headless.sqlite3").await;
    let (_hcode, hout, herr) = run_headless(
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::Alias,
            value: "customers".into(),
            column: None,
            reason: None,
            profile: None,
        },
        &runtime,
        &headless_store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(_hcode, 0, "headless stderr: {herr}");
    assert_eq!(sout, hout, "slash and headless text output agree");

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
    let slash_items = slash_store.knowledge_for_object(&object).await.unwrap();
    let headless_items = headless_store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(
        slash_items[0].id, headless_items[0].id,
        "slash and headless /remember produced different claim ids in store"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 4. /forget and `saya contracts forget` both tombstone; recall excludes both.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn forget_slash_and_headless_tombstone_and_exclude_identically() {
    let root = temp_root("forget_parity");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let (_, _, out, _) = run_slash(
        "/remember analytics.public.orders alias customers",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert!(out.contains("remembered alias customers for analytics.public.orders"));
    assert!(!out.contains("ki-"));

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

    // Slash /forget tombstones.
    let (_cmd, fcode, fout, ferr) = run_slash(
        &format!("/forget {id}"),
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(fcode, 0, "/forget stderr: {ferr}");
    assert!(fout.contains("forgotten"), "/forget out: {fout}");

    // Recall (show) no longer lists it.
    let (_, _, show_out, _) = run_slash(
        "/contract analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert!(
        !show_out.contains("customers"),
        "forgotten claim still listed after /forget: {show_out}"
    );

    // The slash /forget translated command equals the headless forget command.
    assert_eq!(
        _cmd,
        ContractsCommand::Forget {
            claim_id: id.to_string(),
            // Slash /forget uses the default reason; the headless default matches.
            reason: ForgetReasonArg::UserRequest,
        }
    );

    // A second store: headless forget must produce the same exclusion.
    let root2 = temp_root("forget_parity_headless");
    let (runtime2, _n) = runtime_at(&root2);
    let store2 = store_at(&root2).await;
    let (_, _, _) = run_headless(
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::Alias,
            value: "customers".into(),
            column: None,
            reason: None,
            profile: None,
        },
        &runtime2,
        &store2,
        RenderFormat::Text,
    )
    .await;
    let identity2 = identity_for(&runtime2, "local");
    let profile2 = ProfileIdentity::parse(&identity2).unwrap();
    let object2 = DatabaseObjectRef::new(
        profile2,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items2 = store2.knowledge_for_object(&object2).await.unwrap();
    let id2 = items2[0].id.as_str();
    run_headless(
        ContractsCommand::Forget {
            claim_id: id2.to_string(),
            reason: ForgetReasonArg::UserRequest,
        },
        &runtime2,
        &store2,
        RenderFormat::Text,
    )
    .await;
    let (_, hshow, _) = run_headless(
        ContractsCommand::Show {
            table: qualified().into(),
            profile: None,
        },
        &runtime2,
        &store2,
        RenderFormat::Text,
    )
    .await;
    assert!(
        !hshow.contains("customers"),
        "headless forget still listed the claim: {hshow}"
    );

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(root2);
}

// ---------------------------------------------------------------------------
// 5. A malformed table is rejected identically by both.
// ---------------------------------------------------------------------------
