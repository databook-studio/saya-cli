#[tokio::test]
async fn confirm_by_short_prefix_confirms_the_intended_claim() {
    let root = temp_root("confirm_prefix");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let id = seed_candidate(&store, &runtime, "orders").await;
    let prefix = short_prefix(&id);

    let (_cmd, code, out, err) = run_slash(
        &format!("/confirm {prefix}"),
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "/confirm stderr: {err}");
    // The confirm op emits ContractChanged with the FULL id and "confirmed".
    assert!(out.contains("confirmed"), "/confirm out: {out}");
    assert!(
        out.contains(id.as_str()),
        "must name the exact claim: {out}"
    );

    // The claim is now Confirmed — recallable via show.
    let (scode, sout, serr) = run_headless(
        ContractsCommand::Show {
            table: qualified().into(),
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(scode, 0, "show stderr: {serr}");
    assert!(sout.contains("confirmed"), "show after confirm: {sout}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2. Rejecting by the short reference rejects exactly that claim; nothing is
//    promoted (no claim becomes recallable).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn reject_by_short_prefix_rejects_and_promotes_nothing() {
    let root = temp_root("reject_prefix");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    let id = seed_candidate(&store, &runtime, "orders").await;
    let prefix = short_prefix(&id);

    let (_cmd, code, out, err) = run_slash(
        &format!("/reject {prefix}"),
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "/reject stderr: {err}");
    assert!(out.contains("rejected"), "/reject out: {out}");
    assert!(
        out.contains(id.as_str()),
        "must name the exact claim: {out}"
    );

    // Rejected → not recallable. Show finds no contract for the object.
    let (scode, sout, serr) = run_headless(
        ContractsCommand::Show {
            table: qualified().into(),
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(scode, 0, "show stderr: {serr}");
    assert!(
        sout.contains("No contract"),
        "rejected claim must not be recallable: {sout}"
    );

    let _ = fs::remove_dir_all(root);
}
