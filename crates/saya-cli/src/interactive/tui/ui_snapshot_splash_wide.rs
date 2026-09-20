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

/// G1 property 4 — the status line keeps `ws:unbound`: unchanged by this
/// slice (pinned by `status_line_names_the_workspace_binding`; asserted
/// here through the real render so the bar and the header cannot drift).
#[test]
fn the_status_line_keeps_ws_unbound() {
    let mut status = fixed_status();
    status.workspace_root = None;
    let buffer = render_buffer(&empty_app(), &status, 80, 24);
    assert!(
        buffer.contains("ws:unbound"),
        "the status bar keeps ws:unbound:\n{buffer}"
    );
}
