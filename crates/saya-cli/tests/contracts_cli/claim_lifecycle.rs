#[tokio::test]
async fn remember_then_show_round_trips_claim_fields() {
    let root = temp_root("round_trip");
    let (runtime, _connections, _name) = runtime_at(&root);
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
    assert!(out.contains("remembered"), "out: {out}");
    assert!(out.contains("confirmed"), "out: {out}");

    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("customers"), "value missing: {out}");
    assert!(out.contains("table_alias"), "kind missing: {out}");
    assert!(out.contains("user_explicit"), "origin missing: {out}");
    assert!(out.contains("confirmed"), "status missing: {out}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2. remember twice -> duplicate with the same fact, no second claim
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_twice_reports_duplicate_with_same_id() {
    let root = temp_root("duplicate");
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
    let (code1, out1, err1) = run(remember.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code1, 0, "stderr: {err1}");
    assert!(
        out1.contains("remembered alias customers for analytics.public.orders (confirmed)"),
        "confirmation names fact and object in words: {out1}"
    );
    assert!(!out1.contains("ki-"), "must contain no raw id: {out1}");

    let (code2, out2, err2) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code2, 0, "stderr: {err2}");
    assert!(
        out2.contains(
            "duplicate of alias customers for analytics.public.orders — already exists (confirmed)"
        ),
        "second must report duplicate naming fact and object: {out2}"
    );
    assert!(
        !out2.contains("ki-"),
        "duplicate must contain no raw id: {out2}"
    );

    // Exactly one knowledge item exists for the object — the duplicate wrote
    // nothing, so the row count did not grow.
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
        "duplicate created a second knowledge item: {items:?}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2b. re-remembering a directive claim with a NEW reason revises it: the
// value is unchanged (so the single-valued slot does not move), but the
// reason the user just stated is written — a user who explained themselves
// must not be ignored. A genuinely identical re-remember (same value AND
// same reason) is still a no-op duplicate. (Open Question 2.)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn remember_again_with_a_new_reason_revises_the_reason() {
    let root = temp_root("reason_revision");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    // First remember: a time-column with a reason.
    let first = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::TimeColumn,
        value: "return_date".into(),
        column: None,
        reason: Some("a rental only counts once it comes back".into()),
        profile: None,
    };
    let (code1, out1, err1) = run(first, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code1, 0, "stderr: {err1}");
    assert!(out1.contains("remembered"), "first remember: {out1}");

    // Second remember: same value, a NEW reason. This is a revision, not a
    // duplicate — the reason is written.
    let second = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::TimeColumn,
        value: "return_date".into(),
        column: None,
        reason: Some("rentals are counted on return for billing".into()),
        profile: None,
    };
    let (code2, out2, err2) = run(second, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code2, 0, "stderr: {err2}");
    assert!(
        out2.contains("remembered"),
        "a new reason revises the claim, not a duplicate: {out2}"
    );
    assert!(
        !out2.contains("duplicate"),
        "a new reason must not read as a duplicate: {out2}"
    );

    // Exactly one row, and it carries the NEW reason.
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
    assert_eq!(items.len(), 1, "revision wrote no new row: {items:?}");
    assert!(
        matches!(
            &items[0].value,
            saya_types::ClaimPayload::DefaultTimeColumn { column, reason, .. }
            if column == "return_date"
            && reason.as_deref() == Some("rentals are counted on return for billing")
        ),
        "the stored reason is the new one: {:?}",
        items[0].value
    );

    // A third remember with the SAME value and SAME reason is a no-op duplicate.
    let third = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::TimeColumn,
        value: "return_date".into(),
        column: None,
        reason: Some("rentals are counted on return for billing".into()),
        profile: None,
    };
    let (code3, out3, err3) = run(third, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code3, 0, "stderr: {err3}");
    assert!(
        out3.contains("duplicate"),
        "identical re-remember is a duplicate: {out3}"
    );

    // A fourth remember with the SAME value and NO reason does NOT erase the
    // stored reason — silence is not "drop the reason," only a *new* reason
    // revises. It reports a duplicate, and the row keeps its reason.
    let fourth = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::TimeColumn,
        value: "return_date".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code4, out4, err4) = run(fourth, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code4, 0, "stderr: {err4}");
    assert!(
        out4.contains("duplicate"),
        "re-stating without a reason is a duplicate, not an erase: {out4}"
    );
    let items_again = store.knowledge_for_object(&object).await.unwrap();
    assert!(matches!(
        &items_again[0].value,
        saya_types::ClaimPayload::DefaultTimeColumn { reason, .. }
        if reason.as_deref() == Some("rentals are counted on return for billing")
    ));

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 3. forget then show no longer lists the claim
// ---------------------------------------------------------------------------
