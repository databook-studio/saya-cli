/// `/mode plan` through the REAL TUI dispatch: the answer lands in the
/// transcript unchanged and the pending question is abandoned — the old
/// token is cancelled and the next begin mints a different question.
#[test]
fn a_context_change_through_the_tui_dispatch_abandons_the_pending_question() {
    let mut fx = TuiHarness::build("tui-abandon");
    let pending = fx.pend("tui abandon");
    fx.dispatch_line("/mode plan");
    let message = fx.last_system_block().expect("the mode answer is said");
    assert!(
        message.contains("Mode: plan"),
        "the dispatch output is unchanged: {message}"
    );
    assert!(
        pending.cancellation().is_cancelled(),
        "the context change through the TUI dispatch cancels the pending token"
    );
    let next = fx
        .session
        .begin_question(&fx.runtime)
        .expect("the next question begins");
    assert!(
        !next.is_same_question(&pending),
        "the next begin mints a different question after the TUI context change"
    );
}

/// `/help` through the REAL TUI dispatch changes nothing: the pending
/// clarification survives with its token live.
#[test]
fn a_noop_through_the_tui_dispatch_preserves_the_pending_question() {
    let mut fx = TuiHarness::build("tui-preserve");
    let pending = fx.pend("tui preserve");
    fx.dispatch_line("/help");
    let message = fx.last_system_block().expect("the help answer is said");
    assert!(
        message.contains("Slash commands:"),
        "help answers through the dispatch: {message}"
    );
    let next = fx
        .session
        .begin_question(&fx.runtime)
        .expect("the next question begins");
    assert!(
        next.is_same_question(&pending),
        "a no-op through the TUI dispatch re-begins the same pending question"
    );
    assert!(
        !next.cancellation().is_cancelled(),
        "the preserved question's token stays live"
    );
}
