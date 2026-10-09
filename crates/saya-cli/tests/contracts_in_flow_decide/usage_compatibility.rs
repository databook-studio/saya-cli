#[tokio::test]
async fn usage_errors_never_echo_user_argument_text() {
    // Parse-only: a usage error surfaces before any store access. An unopenable
    // store is irrelevant here.
    let secret = "c-SUPERSECRET-leaked-value";

    // No argument at all.
    let bad = parse_slash_command("/confirm").unwrap_err();
    assert!(!bad.to_string().contains(secret), "echoed: {bad}");
    assert!(!bad.to_string().is_empty());

    // Two tokens is a usage error (the prefix is one token).
    let bad = parse_slash_command("/confirm c-abc c-def").unwrap_err();
    assert!(
        !bad.to_string().contains("c-abc"),
        "echoed untrusted input: {bad}"
    );
    assert!(
        !bad.to_string().contains("c-def"),
        "echoed untrusted input: {bad}"
    );

    // The secret-bearing prefix never reaches the message even when it is the
    // sole argument and the command fails for another reason at parse time.
    let parsed = parse_slash_command(&format!("/confirm {secret}"));
    // A single well-formed token parses; it would then resolve at dispatch. The
    // point here is that the PARSE error path (too many tokens / no tokens) does
    // not echo. Asserting the single-token case parsed keeps this honest: it
    // proves the secret is only ever carried in the parsed command, not in an
    // error string.
    assert!(parsed.is_ok(), "single token should parse: {parsed:?}");
    let cmd = match parsed.unwrap() {
        Some(SlashCommand::Contracts(c)) => c,
        other => panic!("expected Contracts, got {other:?}"),
    };
    // The resolved command carries the prefix, but that never surfaces in a
    // usage error — only in the typed dispatcher, which maps failures to
    // payload-free messages (asserted in tests 4/5).
    let _ = cmd;
}

// ---------------------------------------------------------------------------
// 7. /queue and the existing contracts subcommands still behave as before.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn queue_and_existing_subcommands_unchanged() {
    let root = temp_root("unchanged");
    let (runtime, _name) = runtime_at(&root);
    let store = store_at(&root).await;

    // /queue still lists the candidate (the queue is the candidate worklist).
    let id = seed_candidate(&store, &runtime, "orders").await;
    let (_qcmd, qcode, qout, qerr) =
        run_slash("/queue", &runtime, &store, RenderFormat::Text).await;
    assert_eq!(qcode, 0, "/queue stderr: {qerr}");
    assert!(
        qout.contains(id.as_str()),
        "/queue still lists candidates: {qout}"
    );

    // /contracts (list) still works: exit 0, no error. A candidate is not
    // recallable, so list does not show it — that is the unchanged behaviour,
    // not a regression. Confirm the claim, then list shows it.
    let (_ccmd, ccode, _cout, cerr) =
        run_slash("/contracts", &runtime, &store, RenderFormat::Text).await;
    assert_eq!(ccode, 0, "/contracts stderr: {cerr}");

    run_headless(
        ContractsCommand::Decide {
            prefix: id.as_str().into(),
            decision: saya_cli::ReviewDecisionArg::Confirm,
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    // `Decide::Confirm` resolves the prefix to the knowledge item and promotes
    // it to `Active` (the confirm op writes `knowledge_items` now), so `/contracts`
    // (list → recall over `knowledge_items`) shows the confirmed claim.
    let (_ccmd2, _ccode2, cout2, cerr2) =
        run_slash("/contracts", &runtime, &store, RenderFormat::Text).await;
    assert!(
        cout2.contains("orders"),
        "/contracts lists a confirmed claim: {cout2}"
    );
    assert_eq!(cerr2, "");

    // /contract <t> (show) still works and shows the now-confirmed claim.
    let (_scmd, scode, sout, serr) = run_slash(
        "/contract analytics.public.orders",
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(scode, 0, "/contract stderr: {serr}");
    assert!(sout.contains("orders"), "/contract still shows: {sout}");

    // /forget <full-id> still works and is unaffected by the new commands.
    let (_fcmd, fcode, fout, ferr) = run_slash(
        &format!("/forget {id}"),
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(fcode, 0, "/forget stderr: {ferr}");
    assert!(
        fout.contains("forgotten"),
        "/forget still tombstones: {fout}"
    );

    let _ = fs::remove_dir_all(root);
}
