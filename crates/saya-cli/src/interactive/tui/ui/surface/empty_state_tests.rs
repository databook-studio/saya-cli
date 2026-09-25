//! The empty state under a short terminal: decoration yields before
//! guidance, in an explicit drop order, and the first screen names the
//! concepts the redesign added. Splash-only tests render
//! `draw_empty_state` directly onto a `TestBackend` at an exact pane
//! height; the composed-screen test goes through the real `ui::draw`.

use super::*;
use crate::interactive::tui::ui_snapshot_tests::{empty_app, fixed_status, render_buffer};

/// The keyboard hint is the last line of the splash, so a clip from the
/// bottom takes it first — the opposite of the drop order.
const HINT: &str = "/ commands     @ tables     ? help     Ctrl+C quit";

/// The concept line the first screen must name once it has room.
const CONCEPT: &str = "Each request becomes a chapter — activity, answer, and results.";

/// The owl's body glyph marks the art: no splash text contains it.
const ART_GLYPH: char = '\u{2588}';

/// Renders the empty state alone at an exact pane size. `profiles` replaces
/// the fixture's demo list; `workspace_bound` drives the workspace paragraph
/// the way `ui::draw` passes it.
fn splash(width: u16, height: u16, profiles: &[&str], workspace_bound: bool) -> String {
    let mut app = empty_app();
    app.profiles = profiles.iter().map(|name| (*name).to_string()).collect();
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend builds");
    terminal
        .draw(|frame| super::draw_empty_state(frame, &app, frame.area(), Some(workspace_bound)))
        .expect("draw completes");
    format!("{}", terminal.backend())
}

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
/// fits, and it stays gone as the squeeze deepens and guidance is kept.
#[test]
fn a_short_terminal_drops_the_art_first() {
    // Full text fits (with the concept line), compact mascot does not.
    let text_only = splash(80, 25, &[], false);
    assert!(
        !text_only.contains(ART_GLYPH),
        "the art is dropped while every line of text still fits:\n{text_only}"
    );
    assert!(
        text_only.contains(NO_DATABASE_HEADLINE) && text_only.contains(HINT),
        "nothing but the art was dropped:\n{text_only}"
    );
    // Deeper squeeze: the art has not come back, the guidance has not left.
    let squeezed = splash(80, 12, &[], false);
    assert!(
        !squeezed.contains(ART_GLYPH),
        "the art stays gone under a deeper squeeze:\n{squeezed}"
    );
    assert!(
        squeezed.contains(NO_DATABASE_HEADLINE),
        "guidance outlives the art:\n{squeezed}"
    );
}

/// With no profiles configured, the guidance is a stated absence, not
/// chrome: it survives a squeeze that drops the example prompts.
#[test]
fn the_no_database_guidance_outlives_the_examples() {
    let buffer = splash(80, 12, &[], true);
    assert!(
        buffer.contains(NO_DATABASE_HEADLINE),
        "the no-database headline is kept:\n{buffer}"
    );
    for step in NO_DATABASE_STEPS {
        assert!(
            buffer.contains(step),
            "the step is kept ({step:?}):\n{buffer}"
        );
    }
    assert!(
        buffer.contains(NO_DATABASE_FOOTER),
        "the footer is kept:\n{buffer}"
    );
    assert!(
        !buffer.contains("try asking"),
        "the examples were dropped by the squeeze:\n{buffer}"
    );
    assert!(buffer.contains(HINT), "the hint is kept too:\n{buffer}");
}

/// The redesign's concepts — chapters, activity, results — are named on the
/// first screen at a normal height, as one line.
#[test]
fn the_first_screen_names_the_chapter_concept() {
    let buffer = splash(80, 30, &[], false);
    assert!(
        buffer.contains(CONCEPT),
        "the first screen names the chapter concept:\n{buffer}"
    );
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
        buffer.contains(NO_DATABASE_HEADLINE),
        "the no-database headline:\n{buffer}"
    );
    for step in NO_DATABASE_STEPS {
        assert!(buffer.contains(step), "the step ({step:?}):\n{buffer}");
    }
    assert!(buffer.contains(NO_DATABASE_FOOTER), "the footer:\n{buffer}");
    for line in NO_WORKSPACE_LINES {
        assert!(
            buffer.contains(line),
            "the workspace line ({line:?}):\n{buffer}"
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

/// The configured-profile path still lists the databases — the drop order
/// must not disturb what the splash says about a present database.
#[test]
fn the_empty_state_is_unchanged_when_a_database_is_configured() {
    let buffer = splash(80, 24, &["analytics", "billing"], true);
    assert!(buffer.contains("databases"), "the state line:\n{buffer}");
    assert!(buffer.contains("analytics"), "the first profile:\n{buffer}");
    assert!(buffer.contains("billing"), "the second profile:\n{buffer}");
    assert!(
        !buffer.contains(NO_DATABASE_HEADLINE),
        "a configured database draws no missing-database guidance:\n{buffer}"
    );
    assert!(buffer.contains(HINT), "the hint:\n{buffer}");
}

/// The concept line is copy on the first screen: it must survive the same
/// narrow-terminal rule the guidance copy is held to, or it clips.
#[test]
fn the_concept_line_fits_a_narrow_terminal() {
    let buffer = splash(64, 30, &[], false);
    assert!(
        buffer.contains(CONCEPT),
        "the concept line fits a 64-column terminal:\n{buffer}"
    );
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
