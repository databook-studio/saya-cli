// ---------------------------------------------------------------------------
// `/continue` and the job controls' one-release spellings, in the TUI
// ---------------------------------------------------------------------------

/// Bare `/continue` answers the one refusal sentence in this session: the
/// built-in engine's turns never pause on a guard, so there is nothing to
/// continue — and nothing is sent as a turn.
#[test]
fn a_bare_continue_answers_nothing_to_continue() {
    let mut fx = ChainFixture::build("continue-bare");

    assert!(matches!(dispatch_line("/continue", &mut fx), Dispatch::Handled));

    let said = fx
        .app
        .transcript
        .blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        said, crate::chat::NOTHING_TO_CONTINUE,
        "the one sentence, and nothing else"
    );
}

/// `/approve` is `/job approve`'s one-release spelling: the note is said the
/// first time the session uses it, and never again.
#[test]
fn a_job_control_says_its_note_once_per_session() {
    let mut fx = ChainFixture::build("job-alias-note");
    let note = crate::slash::JobAlias::Approve.text();

    // The line goes through the real `tick_pending`, as a submitted line does:
    // the note is said where a job command is carried out, not while parsing.
    for line in ["/approve chain-sentinel", "/approve chain-sentinel"] {
        fx.app.pending = Some(line.to_owned());
        drain_through_tick_pending(&mut fx);
        assert!(fx.app.pending.is_none(), "{line} was consumed");
    }
    fx.app.pending = Some("/deny chain-sentinel".to_owned());
    drain_through_tick_pending(&mut fx);

    let said = fx
        .app
        .transcript
        .blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        said.matches(note).count(),
        1,
        "the note is owed once per session, however often the spelling is used: {said}"
    );
    assert!(
        said.starts_with(note),
        "the note comes before the operation's own answer: {said}"
    );
    assert_eq!(
        said.matches(crate::slash::JobAlias::Deny.text()).count(),
        1,
        "another spelling owes its own note: {said}"
    );
}
