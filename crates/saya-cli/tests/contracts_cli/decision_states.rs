#[tokio::test]
async fn decide_confirm_on_candidate_confirms_and_on_confirmed_conflicts() {
    let root = temp_root("decide_confirm");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // Cache a schema that names `orders` so a re-confirm of the now-`Active`
    // alias has a live table to revalidate against (the alias's `Table`
    // binding is valid whenever the table exists). `store_at` cached an empty
    // default; overwrite it for this profile.
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema())
        .await
        .unwrap();

    let candidate_id = seed_candidate(&store, &runtime, "orders").await;
    // The full stored id is a prefix of itself, so `decide` resolves it to
    // exactly one claim.
    let prefix = candidate_id.as_str().to_string();

    // 1. Pending → Active: the candidate is confirmed.
    let confirm = ContractsCommand::Decide {
        prefix: prefix.clone(),
        decision: ReviewDecisionArg::Confirm,
        profile: None,
    };
    let (code, out, err) = run(confirm, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("confirmed"), "candidate -> confirmed: {out}");

    // 2. Re-confirming the now-Active item against the valid cached schema
    //    revalidates and stays Active — confirm is idempotent, not a conflict.
    //    A re-confirm is how a user asks "is this still true?", and refusing
    //    would leave them no way to re-check a fact they suspect has drifted.
    let (code, out, err) = run(
        ContractsCommand::Decide {
            prefix: prefix.clone(),
            decision: ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "re-confirm must succeed, not conflict: {out}{err}");
    assert!(
        out.contains("confirmed"),
        "re-confirmed item stays Active/confirmed: {out}"
    );

    // 3. Confirming a `Dismissed` item is a typed conflict — a withdrawn fact
    //    is not revivable by revalidation, so confirm refuses rather than
    //    silently reviving it.
    let dismissed_id = seed_candidate(&store, &runtime, "shipments").await;
    let _ = run(
        ContractsCommand::Forget {
            claim_id: dismissed_id.as_str().into(),
            reason: ForgetReasonArg::Incorrect,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (code, out, err) = run(
        ContractsCommand::Decide {
            prefix: dismissed_id.as_str().into(),
            decision: ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(
        code, 0,
        "confirming a dismissed item must not succeed: {out}{err}"
    );
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("conflict"),
        "expected a typed conflict for a dismissed item, got: {combined}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 6. unknown --profile -> typed error listing available names, non-zero
// ---------------------------------------------------------------------------
