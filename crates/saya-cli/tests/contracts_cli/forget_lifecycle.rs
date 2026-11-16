#[tokio::test]
async fn forget_then_show_no_longer_lists_the_claim() {
    let root = temp_root("forget");
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
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        out.contains("remembered alias customers for analytics.public.orders (confirmed)"),
        "names fact and object: {out}"
    );
    assert!(!out.contains("ki-"), "no raw id in remember output: {out}");

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

    let forget = ContractsCommand::Forget {
        claim_id: id.into(),
        reason: ForgetReasonArg::Incorrect,
    };
    let (code, out, err) = run(forget, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("forgotten"), "out: {out}");

    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        !out.contains("customers"),
        "forgotten claim still listed: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 4. remember, forget, then remember again -> duplicate whose status is forgotten
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_after_forget_reports_duplicate_forgotten_not_success() {
    let root = temp_root("reforget");
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
    let (code, out, err) = run(remember.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("remembered"), "out: {out}");
    assert!(!out.contains("ki-"), "no raw id: {out}");

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

    let forget = ContractsCommand::Forget {
        claim_id: id.into(),
        reason: ForgetReasonArg::Incorrect,
    };
    let (code, _out, err) = run(forget, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");

    // Re-remembering the same value hits the tombstone's dedup key and must read
    // as a duplicate of a *forgotten* claim, not as a fresh success or a new id.
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("duplicate"), "out: {out}");
    assert!(
        out.contains("previously forgotten"),
        "duplicate of a forgotten claim must say previously forgotten: {out}"
    );
    assert!(
        out.contains("alias customers for analytics.public.orders"),
        "names fact and object: {out}"
    );
    assert!(!out.contains("ki-"), "no raw id: {out}");
    assert!(
        !out.starts_with("remembered"),
        "must not read as success: {out}"
    );

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
    assert_eq!(
        items.len(),
        1,
        "re-remember created a new knowledge item: {items:?}"
    );
    // The duplicate wrote nothing, so the tombstone stayed dismissed — a
    // re-remember of a forgotten fact does not silently revive it.
    assert_eq!(
        items[0].state,
        KnowledgeState::Dismissed,
        "the forgotten tombstone must stay dismissed: {items:?}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 5. decide --decision confirm: a `Pending` candidate becomes `Active`
//    (confirmed); re-confirming the now-`Active` item against a valid cached
//    schema *revalidates* and stays `Active` (confirm is idempotent —
//    re-confirming is how a user asks "is this still true?"); confirming a
//    `Dismissed` item is a typed conflict (a withdrawn fact is not revivable
//    by revalidation). This is the same confirm op `review --confirm` reached;
//    `decide` resolves the prefix to the id and forwards to it.
// ---------------------------------------------------------------------------
