#[tokio::test]
async fn queue_lists_candidates_and_decide_transitions_them() {
    let root = temp_root("queue_decide");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // `store_at` cached an empty default schema for this profile; drop it so the
    // candidate genuinely has no cached schema and the queue reads the honest
    // `live_schema_unavailable` — matching how `list`/`show`'s no-cache test sets
    // up. An empty cached tree would otherwise read `stale` (the object is
    // absent from it), which is correct but not what this test exercises.
    let identity = identity_for(&runtime, "local");
    store.invalidate_schema(&identity).await.unwrap();

    let cand_id = seed_candidate(&store, &runtime, "orders").await;
    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");
    // The queue is a worklist: each entry carries the decision a reviewer is
    // being asked to make as a status word. A fresh candidate reads
    // `candidate`; a persisted stale claim reads `stale`. The fields a
    // reviewer needs are the id, the status, the kind/value, the object, and
    // the schema state.
    assert!(
        out.contains(cand_id.as_str()),
        "queue must name the candidate's full id: {out}"
    );
    assert!(out.contains("candidate"), "queue must show status: {out}");
    assert!(out.contains("table_alias"), "queue must show kind: {out}");
    assert!(out.contains("customers"), "queue must show value: {out}");
    assert!(
        out.contains("analytics.public.orders"),
        "queue must show object: {out}"
    );
    assert!(
        out.contains("live_schema_unavailable"),
        "queue must show schema state: {out}"
    );
    assert!(
        !out.contains("evidence 0"),
        "queue must not show a placeholder evidence count: {out}"
    );

    // Confirm the candidate: it becomes recallable (show lists it) and leaves
    // the queue on the next read. `decide` resolves the full id (a prefix of
    // itself) to the candidate and forwards to the `confirm` op.
    let confirm = ContractsCommand::Decide {
        prefix: cand_id.as_str().into(),
        decision: ReviewDecisionArg::Confirm,
        profile: None,
    };
    let (code, out, err) = run(confirm, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "confirm stderr: {err}");
    assert!(out.contains("confirmed"), "confirm out: {out}");

    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("customers"),
        "confirmed candidate is now recallable: {out}"
    );

    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");
    assert!(
        !out.contains(cand_id.as_str()),
        "confirmed candidate must leave the queue: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_rejected_candidate_never_becomes_recallable() {
    let root = temp_root("queue_reject");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let cand_id = seed_candidate(&store, &runtime, "orders").await;
    let reject = ContractsCommand::Decide {
        prefix: cand_id.as_str().into(),
        decision: ReviewDecisionArg::Reject,
        profile: None,
    };
    let (code, out, err) = run(reject, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "reject stderr: {err}");
    assert!(out.contains("rejected"), "reject out: {out}");

    // The rejected candidate left the queue…
    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");
    assert!(
        !out.contains(cand_id.as_str()),
        "rejected candidate must leave the queue: {out}"
    );

    // …and never becomes recallable: show finds no contract for the object.
    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("No contract"),
        "rejected candidate must never be recallable: {out}"
    );

    let _ = fs::remove_dir_all(root);
}
