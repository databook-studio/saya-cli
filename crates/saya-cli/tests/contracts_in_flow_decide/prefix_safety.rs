#[tokio::test]
async fn ambiguous_or_unresolvable_prefix_refuses_and_changes_nothing() {
    let root = temp_root("ambiguous_prefix");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    // Seed two candidates whose ids share the `ki-` marker; a bare "ki-" prefix
    // matches both → ambiguous. (Both ids start with "ki-" by construction.)
    let id_a = seed_candidate(&store, &runtime, "orders").await;
    let id_b = seed_candidate(&store, &runtime, "returns").await;
    assert_ne!(id_a, id_b);

    // The shared prefix both ids start with is "ki-": ambiguous.
    let ambiguous = "ki-";
    let (acode, aout, aerr) = run_headless(
        ContractsCommand::Decide {
            prefix: ambiguous.into(),
            decision: saya_cli::ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(acode, 0, "ambiguous prefix must not succeed: {aout}{aerr}");
    let combined = format!("{aout}{aerr}");
    // The refusal names why without echoing the typed prefix: it says the
    // reference matches more than one claim (the "unambiguous or refused"
    // invariant), never the prefixes themselves.
    assert!(
        combined.contains("more than one claim"),
        "ambiguous refusal must name why: {combined}"
    );
    assert!(
        !combined.contains(ambiguous),
        "refusal echoed the prefix: {combined}"
    );
    // Neither item was changed: both still Pending.
    for id in [&id_a, &id_b] {
        let item = store
            .get_knowledge_item(id.as_str())
            .await
            .unwrap()
            .expect("item present");
        assert_eq!(
            item.state,
            KnowledgeState::Pending,
            "ambiguous changed {id}"
        );
    }

    // An unresolvable prefix (no item starts with it) refuses too.
    let (ncode, nout, nerr) = run_headless(
        ContractsCommand::Decide {
            prefix: "ki-zzzzz".into(),
            decision: saya_cli::ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        ncode, 0,
        "unresolvable prefix must not succeed: {nout}{nerr}"
    );
    let ncombined = format!("{nout}{nerr}");
    assert!(
        ncombined.contains("no claim matches"),
        "unresolvable refusal must name why: {ncombined}"
    );
    assert!(
        !ncombined.contains("ki-zzzzz"),
        "unresolvable refusal echoed the prefix: {ncombined}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 5. A stale reference behaves as §4 decided — asserted explicitly.
//    The prefix is a stored id; ids are immutable, so "stale" cannot mean the
//    prefix now points at a *different* claim (the silent-wrong-write failure
//    mode a per-turn index has). It means one of:
//    (a) the prefix is too short / matches more than one claim → AMBIGUOUS,
//        refused, nothing changes (covered above); and
//    (b) the prefix matched a claim that no longer exists (forgotten) → NOT
//        FOUND, refused, nothing changes.
//    The defence: a prefix never silently resolves to a different claim than
//    the one the user saw — it resolves to the same claim or refuses.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stale_prefix_refuses_rather_than_write_the_wrong_claim() {
    let root = temp_root("stale_prefix");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    // Seed two candidates; the user saw the first receipt and remembers only
    // "ki-" (the shortest thing that could be on screen). Another claim now
    // shares that prefix. A per-turn index would silently confirm the WRONG
    // one; the prefix refuses because it is ambiguous.
    let _id_a = seed_candidate(&store, &runtime, "orders").await;
    let id_b = seed_candidate(&store, &runtime, "returns").await;
    let (code, out, err) = run_headless(
        ContractsCommand::Decide {
            prefix: "ki-".into(),
            decision: saya_cli::ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(code, 0, "ambiguous stale prefix must refuse: {out}{err}");
    // Both still Pending — nothing written to durable memory.
    for id in [&_id_a, &id_b] {
        let item = store
            .get_knowledge_item(id.as_str())
            .await
            .unwrap()
            .expect("item present");
        assert_eq!(item.state, KnowledgeState::Pending);
    }

    // (b) The prefix matched an item that has since been forgotten: the row is
    // still present (Dismissed), so the prefix still resolves, but `confirm`
    // refuses a dismissed item as a conflict — nothing is promoted. Forget
    // id_b, then try to confirm it by its full prefix.
    run_headless(
        ContractsCommand::Forget {
            claim_id: id_b.as_str().into(),
            reason: ForgetReasonArg::UserRequest,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (fcode, fout, ferr) = run_headless(
        ContractsCommand::Decide {
            prefix: id_b.as_str().into(),
            decision: saya_cli::ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        fcode, 0,
        "forgotten item's prefix must not confirm: {fout}{ferr}"
    );
    // A forgotten item is not promoted to Active by this path.
    let item = store
        .get_knowledge_item(id_b.as_str())
        .await
        .unwrap()
        .expect("tombstone present");
    assert_ne!(
        item.state,
        KnowledgeState::Active,
        "forgotten was confirmed"
    );

    let _ = fs::remove_dir_all(root);
}
