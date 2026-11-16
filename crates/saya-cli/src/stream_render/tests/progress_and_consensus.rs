/// A progress-only event must render to nothing, not to
/// `unrecognized agent event`. The catch-all is deliberately loud so a
/// content-bearing variant cannot be dropped silently, which means every
/// contentless one needs an explicit arm — this has been missed three times
/// (`KnowledgeLearningSkipped`, `KnowledgeProposed`, and
/// `KnowledgeLearningStarted`, which printed the error string during a live
/// `saya ask` while the whole suite was green).
#[test]
fn progress_only_events_render_to_nothing_not_to_an_error() {
    assert!(
        terminal_event(AgentEvent::KnowledgeLearningStarted).is_none(),
        "a progress signal must not reach the headless renderer"
    );

    // And the loud path still works for a variant that does carry content.
    let complete = terminal_event(AgentEvent::Complete).expect("Complete renders");
    assert!(
        !matches!(complete, TerminalEvent::NotImplemented { .. }),
        "a known content event must not fall through to the catch-all"
    );
    assert!(
        terminal_event(AgentEvent::answering_budget_stopped(
            BudgetDimension::ElapsedTime
        ))
        .is_none()
    );
}

/// `ReasoningText` carries content (chain-of-thought), so by its nature it
/// would reach the loud catch-all and print
/// `Not implemented: unrecognized agent event` under a correct answer in the
/// headless `saya ask` path — exactly the regression that shipped green
/// three times. The headless renderer displays nothing for reasoning: it is
/// a *scope* decision (display belongs to the interactive transcript, not a
/// pipe) rather than a *nature-of-the-event* decision (reasoning is content,
/// not progress). The assertion pins the scope choice so a future reader
/// cannot conclude reasoning is progress.
#[test]
fn reasoning_text_renders_to_nothing_in_the_headless_path() {
    assert!(
        terminal_event(AgentEvent::reasoning_text("I considered the time column")).is_none(),
        "reasoning must not reach the headless renderer, and must not fall \
         through to the `unrecognized agent event` catch-all"
    );
}

/// The fix is not a blanket silence. `ReasoningText` renders to `None`, but
/// a content event the headless renderer *does* understand still reaches a
/// real `TerminalEvent` and never the `NotImplemented` catch-all. Without
/// this, silencing reasoning by widening the catch-all would pass the test
/// above and quietly break every other variant.
#[test]
fn silencing_reasoning_does_not_silence_a_content_event() {
    // `AssistantText` is the variant `ReasoningText` mirrors — content, and
    // the headless renderer must still surface it.
    let text = terminal_event(AgentEvent::assistant_text("the answer")).expect("renders");
    assert!(
        matches!(text, TerminalEvent::AssistantText { .. }),
        "a content event must reach a real TerminalEvent, not be silenced: {text:?}"
    );
    // And `Complete` — the other content-bearing terminator — still renders.
    let complete = terminal_event(AgentEvent::complete()).expect("renders");
    assert!(
        !matches!(complete, TerminalEvent::NotImplemented { .. }),
        "Complete must not fall through to the catch-all: {complete:?}"
    );
}

/// The designated answering SQL reaches the NDJSON stream under its own type
/// tag so a harness can pair the prose answer with its query, and the text
/// adapter stays silent (the SQL was already shown when the query ran).
#[test]
fn answer_designated_reaches_ndjson_and_stays_silent_in_text() {
    let event = AgentEvent::answer_designated("SELECT count(*) FROM t");
    let terminal = terminal_event(event).expect("designation renders headlessly");
    let json = render_event(&terminal, RenderFormat::Ndjson);
    assert!(
        json.stdout.contains(r#""event":"answer_designated""#),
        "ndjson must tag the designation: {json:?}"
    );
    assert!(
        json.stdout.contains("SELECT count(*) FROM t"),
        "ndjson must carry the SQL: {json:?}"
    );
    let text = render_event(&terminal, RenderFormat::Text);
    assert!(
        text.stdout.is_empty(),
        "the text adapter must not echo the designation: {text:?}"
    );
}

/// The consensus decision reaches the NDJSON stream under its own type tag
/// (so a harness can read the vote tallies) and prints a text line naming
/// the believed query — never falling through to the `unrecognized agent
/// event` catch-all. Carries the SQL only; no result rows.
#[test]
fn consensus_decided_reaches_ndjson_and_names_the_winner_in_text() {
    let event = AgentEvent::consensus_decided(
        Some("SELECT count(*) FROM t".into()),
        3,
        3,
        3,
        3,
        false,
        false,
    );
    let terminal = terminal_event(event).expect("consensus renders headlessly");
    assert!(
        !matches!(terminal, TerminalEvent::NotImplemented { .. }),
        "ConsensusDecided must not fall through to the catch-all: {terminal:?}"
    );
    let json = render_event(&terminal, RenderFormat::Ndjson);
    assert!(
        json.stdout.contains(r#""event":"consensus_decided""#),
        "ndjson must tag the consensus: {json:?}"
    );
    assert!(
        json.stdout.contains("SELECT count(*) FROM t"),
        "ndjson must carry the winning SQL, not result rows: {json:?}"
    );
    assert!(
        json.stdout.contains(r#""attempts":3"#) && json.stdout.contains(r#""votes":3"#),
        "ndjson must carry the tallies: {json:?}"
    );
    let text = render_event(&terminal, RenderFormat::Text);
    assert!(
        text.stdout
            .contains("consensus · 3 attempts, 3 voted, 3 agreed"),
        "the text adapter names the tallies: {text:?}"
    );
    assert!(
        text.stdout.contains("believing: SELECT count(*) FROM t"),
        "the text adapter names the believed query: {text:?}"
    );

    // A tie with no winner names the disagreement honestly, not as a guess.
    let tied = AgentEvent::consensus_decided(None, 3, 3, 1, 0, true, false);
    let tied_terminal = terminal_event(tied).expect("renders");
    let tied_text = render_event(&tied_terminal, RenderFormat::Text);
    assert!(
        tied_text.stdout.contains("tied, no winner"),
        "a tie with no winner says so: {tied_text:?}"
    );
}
