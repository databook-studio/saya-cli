/// A clap parse with a too-short prefix (`ki-`, two chars) is *accepted* — the
/// floor is enforced at the resolve step, not parse time — but a bare empty
/// prefix is also accepted by clap and refused at resolve. This documents that
/// `decide`'s illegal-input story is "unambiguous or refused" at runtime, the
/// property that retires `review`'s runtime `AmbiguousReview`. (Parse-time
/// refusal of the *decision* value is exercised by the ValueEnum below.)
#[tokio::test]
async fn decide_refuses_an_ambiguous_prefix_at_runtime_not_parse_time() {
    let root = temp_root("decide_ambiguous_prefix");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    let _a = seed_candidate(&store, &runtime, "orders").await;
    let _b = seed_candidate(&store, &runtime, "returns").await;

    // `ki-` matches both candidates → ambiguous at the resolve step. clap does
    // not reject it (a two-char prefix parses); the dispatcher refuses.
    let (code, out, err) = run(
        ContractsCommand::Decide {
            prefix: "ki-".into(),
            decision: ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_ne!(code, 0, "ambiguous prefix must refuse: {out}{err}");
    let combined = format!("{out}{err}");
    assert!(
        combined.contains("more than one claim"),
        "ambiguous refusal names why: {combined}"
    );
    assert!(
        !combined.contains("ki-"),
        "refusal must not echo the prefix: {combined}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// the retired `review` subcommand is gone, and its replacement `decide`
// is what `--help` advertises. `review` was undocumented (never in `docs/`) and
// nothing routes to it but the CLI and a pass-through TUI arm; `decide` covers
// every capability it had (proven above). The illegal `--confirm --reject`
// combination `review` caught at runtime is now unexpressible: `decide` takes a
// single `--decision` ValueEnum clap rejects at parse time.
// ---------------------------------------------------------------------------
