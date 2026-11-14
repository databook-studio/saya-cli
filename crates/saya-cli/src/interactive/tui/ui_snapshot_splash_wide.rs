/// Composed-screen behaviour (the splash and wide result tables) as plain
/// assertions, alongside the `insta` snapshots.
use super::support::{empty_app, fixed_status, render_buffer};

// --- Splash and status for the unbound workspace. -------------------------

/// Property 3 — the TUI splash states the absent database beside the
/// unbound workspace: both present, neither dropped, neither made the
/// other's prerequisite. `empty_app` with no profiles paints the
/// database-none line; the unbound status carries no root, so the
/// workspace paragraph paints too.
#[test]
fn splash_names_unbound_beside_no_database() {
    // `empty_app` carries demo profiles, so clear them: the database-none
    // line paints only with no profiles configured.
    let mut app = empty_app();
    app.profiles.clear();
    assert!(
        app.profiles.is_empty(),
        "no profiles, so the database-none line paints"
    );
    let mut status = fixed_status();
    status.workspace_root = None;
    let buffer = render_buffer(&app, &status, 80, 30);
    assert!(
        buffer.contains("none yet — optional"),
        "the database-none line is not dropped:\n{buffer}"
    );
    for fragment in [
        "No workspace is bound",
        "file tools are",
        "--workspace <dir>",
    ] {
        assert!(
            buffer.contains(fragment),
            "the unbound-workspace line is present beside it ({fragment:?}):\n{buffer}"
        );
    }
}

/// Bound control — a bound session's splash draws no workspace paragraph: the
/// unbound lines are absent, so bound output stays byte-identical to the
/// unbound case's lines removed.
#[test]
fn splash_stays_silent_when_a_workspace_is_bound() {
    // `fixed_status` is the bound case (it carries a root); keep the demo
    // profiles too, so both paragraphs are in their silent shape.
    let app = empty_app();
    let buffer = render_buffer(&app, &fixed_status(), 80, 30);
    assert!(
        !buffer.contains("No workspace is bound"),
        "a bound session draws no workspace paragraph:\n{buffer}"
    );
}

/// Property 4 — an unbound workspace stays on screen: the top context
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
