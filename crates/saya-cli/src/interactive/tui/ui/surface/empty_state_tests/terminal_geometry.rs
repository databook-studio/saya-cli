use super::*;

/// A short terminal clips the splash from the bottom, and the hint is the
/// last line — so the first thing to vanish was how to do anything at all.
/// The drop order must keep the hint to the end.
#[test]
fn a_short_terminal_keeps_the_keyboard_hint() {
    let buffer = splash(80, 14, &[], false);
    assert!(
        buffer.contains(HINT),
        "the hint survives a terminal that cannot fit everything:\n{buffer}"
    );
}
/// The art is pure decoration: it is gone while every line of text still
/// fits, and it stays gone as the squeeze deepens and the state lines are
/// kept.
#[test]
fn a_short_terminal_drops_the_art_first() {
    // Full text fits (with the concept line), compact mascot does not.
    let text_only = splash(80, 25, &[], false);
    assert!(
        !text_only.contains(ART_GLYPH),
        "the art is dropped while every line of text still fits:\n{text_only}"
    );
    assert!(
        text_only.contains("AI        ready") && text_only.contains(HINT),
        "nothing but the art was dropped:\n{text_only}"
    );
    // Deeper squeeze: the art has not come back, the state has not left.
    let squeezed = splash(80, 12, &[], false);
    assert!(
        !squeezed.contains(ART_GLYPH),
        "the art stays gone under a deeper squeeze:\n{squeezed}"
    );
    assert!(
        squeezed.contains("AI        ready") && squeezed.contains(DATABASE_NONE),
        "the state lines outlive the art:\n{squeezed}"
    );
}
/// The three state lines are a stated absence, not chrome: they survive a
/// squeeze that drops the example prompts.
#[test]
fn the_three_state_lines_outlive_the_examples() {
    let buffer = splash(80, 12, &[], true);
    assert!(
        buffer.contains("AI        ready"),
        "the AI readiness line is kept:\n{buffer}"
    );
    assert!(
        buffer.contains(PROJECT_READY),
        "the project readiness line is kept:\n{buffer}"
    );
    assert!(
        buffer.contains(DATABASE_NONE),
        "the database line is kept:\n{buffer}"
    );
    assert!(
        !buffer.contains("try asking"),
        "the examples were dropped by the squeeze:\n{buffer}"
    );
    assert!(buffer.contains(HINT), "the hint is kept too:\n{buffer}");
}
/// On a short terminal the entry choices shed first — the three state
/// lines and the hint are what outlive everything.
#[test]
fn short_terminal_drops_the_choices_before_the_state_lines() {
    let buffer = splash(80, 12, &["analytics", "billing"], false);
    assert!(
        buffer.contains("AI        ready"),
        "the AI line survives the squeeze:\n{buffer}"
    );
    assert!(
        buffer.contains("Database  analytics  ·  billing"),
        "the database line survives the squeeze:\n{buffer}"
    );
    for line in NO_WORKSPACE_LINES {
        assert!(
            buffer.contains(line),
            "the workspace line survives the squeeze ({line:?}):\n{buffer}"
        );
    }
    assert!(
        !buffer.contains(START_HEADER),
        "the choices drop before the state lines:\n{buffer}"
    );
    assert!(
        !buffer.contains("try asking"),
        "the examples drop too:\n{buffer}"
    );
    assert!(buffer.contains(HINT), "the hint survives:\n{buffer}");
}
/// Nothing is dropped when there is room — the fix must not over-trim.
#[test]
fn a_tall_terminal_still_shows_everything() {
    let buffer = splash(80, 40, &[], false);
    assert!(
        buffer.contains(ART_GLYPH),
        "the mascot paints when there is room:\n{buffer}"
    );
    assert!(buffer.contains("◆ saya"), "the splash title:\n{buffer}");
    assert!(
        buffer.contains("Ask your databases in plain language."),
        "the tagline:\n{buffer}"
    );
    assert!(
        buffer.contains("AI        ready"),
        "the AI readiness line:\n{buffer}"
    );
    assert!(
        buffer.contains(DATABASE_NONE),
        "the optional-absence database line:\n{buffer}"
    );
    for line in NO_WORKSPACE_LINES {
        assert!(
            buffer.contains(line),
            "the workspace line ({line:?}):\n{buffer}"
        );
    }
    assert!(
        buffer.contains(START_HEADER),
        "the choices header:\n{buffer}"
    );
    for choice in [
        START_CODE_UNBOUND,
        START_SETUP,
        START_QUERY,
        START_OPEN,
        START_AI,
    ] {
        assert!(
            buffer.contains(choice),
            "the choice ({choice:?}):\n{buffer}"
        );
    }
    assert!(buffer.contains(CONCEPT), "the concept line:\n{buffer}");
    assert!(
        buffer.contains("try asking"),
        "the examples header:\n{buffer}"
    );
    for prompt in [
        "which tables track billing?",
        "top 5 customers by revenue",
        "compare row counts across the connected databases",
    ] {
        assert!(
            buffer.contains(prompt),
            "the prompt ({prompt:?}):\n{buffer}"
        );
    }
    assert!(buffer.contains(HINT), "the hint:\n{buffer}");
}
/// The whole composed frame — context line, splash pane, status bar, input
/// box — keeps the hint when the terminal is short. This is the user-visible
/// shape of the fix, pinned once through the real `ui::draw`.
#[test]
fn the_composed_first_screen_keeps_the_hint_on_a_short_terminal() {
    let mut app = empty_app();
    app.profiles.clear();
    let mut status = fixed_status();
    status.workspace_root = None;
    let buffer = render_buffer(&app, &status, 80, 24);
    assert!(
        buffer.contains(HINT),
        "the composed screen keeps the hint on a short terminal:\n{buffer}"
    );
    insta::assert_snapshot!(buffer);
}
