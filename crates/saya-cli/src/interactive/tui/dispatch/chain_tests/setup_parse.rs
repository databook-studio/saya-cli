/// `/setup` through the real dispatch: the ONE new outcome, `Dispatch::RunSetup`
/// — the dispatch never runs the wizard itself (the loop's `tick_pending` owns
/// the handoff under the busy gate), it only routes the request.
#[test]
fn setup_dispatch_returns_run_setup() {
    let mut fx = ChainFixture::build("setup-dispatch");
    assert!(
        matches!(dispatch_line("/setup", &mut fx), Dispatch::RunSetup),
        "the real dispatch of /setup must route the handoff"
    );
}

/// `/setup now` is a parse error, not an argument: an Error block with the
/// pinned wording, and a Handled outcome.
#[test]
fn setup_with_trailing_text_is_a_parse_error_block() {
    let mut fx = ChainFixture::build("setup-parse-error");
    assert!(
        matches!(dispatch_line("/setup now", &mut fx), Dispatch::Handled),
        "the parse error is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("a parse error is an error block");
    assert_eq!(
        error, "setup takes no argument: /setup",
        "the pinned parse-error wording: {error}"
    );
}
