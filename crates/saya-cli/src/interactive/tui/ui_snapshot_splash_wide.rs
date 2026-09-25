/// Composed-screen behaviour snapshots (splash parity and wide result tables), moved byte-identical from the hub.
/// Plain assertions only — the five `insta` snapshots stay in the hub.
use super::support::{empty_app, fixed_status, render_buffer};

// --- Slice 2 (G1): splash/status parity for the unbound workspace. --------

/// G1 property 3 — the TUI splash shows the unbound-workspace line beside
/// the no-database lines: both present, neither dropped. `empty_app` has no
/// profiles (so the no-database guidance paints) and the unbound status
/// carries no root (so the workspace paragraph paints too).
#[test]
fn splash_names_unbound_beside_no_database() {
    use crate::interactive::tui::ui::surface::{NO_DATABASE_HEADLINE, NO_WORKSPACE_LINES};
    // `empty_app` carries demo profiles, so clear them: the no-database
    // guidance paints only with no profiles configured.
    let mut app = empty_app();
    app.profiles.clear();
    assert!(
        app.profiles.is_empty(),
        "no profiles, so the no-database guidance paints"
    );
    let mut status = fixed_status();
    status.workspace_root = None;
    let buffer = render_buffer(&app, &status, 80, 30);
    assert!(
        buffer.contains(NO_DATABASE_HEADLINE),
        "the no-database headline is not dropped:\n{buffer}"
    );
    for line in NO_WORKSPACE_LINES {
        assert!(
            buffer.contains(line),
            "the unbound-workspace line is present beside it ({line:?}):\n{buffer}"
        );
    }
}

/// Slice 2, bound control — a bound session's splash draws no workspace
/// paragraph: the unbound lines are absent, so bound output stays
/// byte-identical to before this slice.
#[test]
fn splash_stays_silent_when_a_workspace_is_bound() {
    use crate::interactive::tui::ui::surface::NO_WORKSPACE_LINES;
    // `fixed_status` is the bound case (it carries a root); keep the demo
    // profiles too, so both paragraphs are in their silent shape.
    let app = empty_app();
    let buffer = render_buffer(&app, &fixed_status(), 80, 30);
    for line in NO_WORKSPACE_LINES {
        assert!(
            !buffer.contains(line),
            "a bound session draws no workspace paragraph ({line:?}):\n{buffer}"
        );
    }
}

/// G1 property 4 — an unbound workspace stays on screen: the top context
/// line states `No workspace bound`, and the bottom bar (which names only
/// the database and model since the split-by-job redesign) never carries a
/// `ws:` token. Asserted through the real render.
#[test]
fn an_unbound_workspace_is_stated_on_the_context_line() {
    let mut status = fixed_status();
    status.workspace_root = None;
    let buffer = render_buffer(&empty_app(), &status, 80, 24);
    assert!(
        buffer.contains("No workspace bound"),
        "the context line states the unbound workspace:\n{buffer}"
    );
    assert!(
        !buffer.contains("ws:"),
        "the bottom bar no longer carries ws: tokens:\n{buffer}"
    );
}
