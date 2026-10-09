#[tokio::test]
async fn contracts_list_slash_and_headless_agree_on_claim_ids_and_order() {
    let root = temp_root("list_parity");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    // Seed two distinct claims so order is observable, through the headless path.
    for value in ["customers", "orders_alias"] {
        let (code, out, err) = run_headless(
            ContractsCommand::Remember {
                table: qualified().into(),
                kind: ClaimKindArg::Alias,
                value: value.into(),
                column: None,
                reason: None,
                profile: None,
            },
            &runtime,
            &store,
            RenderFormat::Text,
        )
        .await;
        assert_eq!(code, 0, "seed stderr: {err}");
        assert!(out.starts_with("remembered"), "seed out: {out}");
    }

    let headless = run_headless(
        ContractsCommand::List { profile: None },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    let (cmd, code, out, err) = run_slash("/contracts", &runtime, &store, RenderFormat::Text).await;

    assert_eq!(code, 0, "/contracts stderr: {err}");
    // The translated slash command must equal the headless one — same operation,
    // same `profile: None` (the active-profile stamp is the TUI adapter's job).
    assert_eq!(cmd, ContractsCommand::List { profile: None });
    // The rendered stanzas — claim id lines and their order — must match byte
    // for byte. A second DTO mapping or a second recall would diverge here.
    assert_eq!(
        out, headless.1,
        "slash /contracts diverged from headless list"
    );
    assert_eq!(err, headless.2);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 2. /contracts <t> (and its /contract alias) and `saya contracts show <t>`
//    produce the same contract. A later change merged `/contracts` and `/contract` into one
//    command whose optional argument selects the operation; this asserts the
//    merged spelling translates to the same `Show` the headless parser builds,
//    and that the kept `/contract` alias produces byte-identical output.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn contract_show_slash_and_headless_agree_on_contract() {
    let root = temp_root("show_parity");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    run_headless(
        ContractsCommand::Remember {
            table: qualified().into(),
            kind: ClaimKindArg::Alias,
            value: "customers".into(),
            column: None,
            reason: None,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;

    let headless = run_headless(
        ContractsCommand::Show {
            table: qualified().into(),
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;

    // The merged spelling: `/contracts <table>` → Show.
    let (merged_cmd, merged_code, merged_out, merged_err) = run_slash(
        "/contracts analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(merged_code, 0, "/contracts <table> stderr: {merged_err}");
    assert_eq!(
        merged_cmd,
        ContractsCommand::Show {
            table: qualified().into(),
            profile: None,
        }
    );
    // Same object, schema state, claim ids, conflicts — the whole rendered stanza.
    assert_eq!(
        merged_out, headless.1,
        "/contracts <table> diverged from headless show"
    );
    assert_eq!(merged_err, headless.2);

    // The kept `/contract` alias must produce byte-identical output to the merged
    // spelling and to headless — it is the same operation, not a second one.
    let (alias_cmd, alias_code, alias_out, alias_err) = run_slash(
        "/contract analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(alias_code, 0, "/contract alias stderr: {alias_err}");
    assert_eq!(
        alias_cmd, merged_cmd,
        "/contract alias translated to a different command"
    );
    assert_eq!(
        alias_out, merged_out,
        "/contract alias diverged from /contracts"
    );
    assert_eq!(alias_err, merged_err);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 3. /remember and `saya contracts remember` produce the *same* claim id —
//    proving both go through the same deterministic identity and payload
//    constructors. Two fresh stores in the same root share the connections path
//    (the cache scope), so the derived profile identity is identical and each
//    path stores a *new* claim whose id can be compared (not a duplicate).
// ---------------------------------------------------------------------------
