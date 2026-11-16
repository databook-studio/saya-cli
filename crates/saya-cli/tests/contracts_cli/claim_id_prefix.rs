/// `decide` accepts a full-length claim id as a prefix of itself and resolves
/// it to exactly one claim. `review` took the full id;
/// `decide` takes a prefix — a full id is the degenerate prefix that matches
/// only itself.
#[tokio::test]
async fn decide_accepts_a_full_length_claim_id_as_a_prefix_of_itself() {
    let root = temp_root("decide_full_id_prefix");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // Cache a schema that names `orders` so the confirm revalidation (confirm
    // is idempotent on an Active alias whose table exists) succeeds.
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema())
        .await
        .unwrap();

    let candidate_id = seed_candidate(&store, &runtime, "orders").await;
    // The FULL stored id (a `ki-…` of 67 chars), not an abbreviation.
    let full_id = candidate_id.as_str().to_string();

    let decide = ContractsCommand::Decide {
        prefix: full_id.clone(),
        decision: ReviewDecisionArg::Confirm,
        profile: None,
    };
    let (code, out, err) = run(decide, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "a full-length id must resolve via decide: {err}");
    assert!(out.contains("confirmed"), "candidate -> confirmed: {out}");

    // It resolved to *exactly* one claim: the candidate is now Active, and no
    // other item changed. There was only one seeded item, so this is the
    // "exactly one" guarantee the prefix floor exists to give a full id.
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
    assert_eq!(items.len(), 1, "decide wrote no new row: {items:?}");
    assert_eq!(
        items[0].state,
        KnowledgeState::Active,
        "the one matched claim is the one that was confirmed: {items:?}"
    );

    let _ = fs::remove_dir_all(root);
}
