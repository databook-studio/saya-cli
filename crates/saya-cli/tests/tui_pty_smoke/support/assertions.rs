/// The positive/negative assertion set every rung shares: the splash must
/// paint and the active profile must appear in the status bar, while none of
/// the fatal-failure strings may appear anywhere on screen. The negatives are
/// the point — a test that only checks the happy string passes on a screen
/// that also contains `Error: local state store is unavailable`, which is
/// exactly the defect this suite hunts.
fn assert_splash_and_status(screen: &vt100::Screen) {
    assert_splash_for_profile(screen, "demo", "screen");
}

/// Same as [`assert_splash_and_status`] but tags failure messages with `label`
/// so a rung that asserts two launches (or names a non-`demo` profile) points
/// at the one that broke.
fn assert_splash_and_status_named(screen: &vt100::Screen, label: &str) {
    assert_splash_for_profile(screen, "demo", label);
}

/// Like [`assert_splash_and_status`] but checks a profile name other than
/// `demo` — used by rung 3, which points at the docker pagila profile.
/// Every rung launches outside any worktree, so the startup trust question is
/// open on the settled screen: the surface paints first (the positives
/// below) and the question rides inside it (the pane positive), never as a raw
/// stdin read in front of it.
///
/// The needles are the NEW surface's words (T19 switched the launch over):
/// the status line's idle word is `? help` (`view::status::ActivityState`), and
/// the mode chip, the database and the model are one row of `│`-separated
/// segments (`view::status_fit::SEP`) where the old splash had an `AI ready`
/// line and a `Database <name>` column. The question and its two exits are the
/// new pane's (`view::trust::{TITLE, ANSWERS}`).
fn assert_splash_for_profile(screen: &vt100::Screen, profile: &str, label: &str) {
    let text = screen_text(screen);

    // Positive: the first screen the REPL paints must be present — asserted on
    // what survives a squeeze, not on the branding. The status line's `? help`
    // is the keyboard hint that must outlive every layout.
    assert_in(
        &text,
        "? help",
        &format!("keyboard hint missing from {label} — it must outlive decoration"),
        &text,
    );

    // Positive: the status line painted end to end — the mode chip, the model
    // this launch configured, then the active profile. The offline config is
    // model `smoke`, so the model segment must be there.
    assert_in(
        &text,
        &format!("read-only │ smoke │ {profile}"),
        &format!("status line (mode · model · database) missing from {label}"),
        &text,
    );

    // Positive: the startup trust question is open — the surface's rendering of
    // the one trust decision, beside the first paint, never before it. The
    // launch dir is outside every worktree with no `--workspace`, so the
    // decision fires; the pane carries the shared prompt's body and both exits.
    assert_in(
        &text,
        "trust this folder?",
        &format!("trust pane title missing from {label}"),
        &text,
    );
    assert_in(
        &text,
        "Trust this folder as the session",
        &format!("trust pane body missing from {label}"),
        &text,
    );
    for exit in ["[y] trust this folder", "[n] continue unbound"] {
        assert_in(
            &text,
            exit,
            &format!("trust pane exit {exit} missing from {label}"),
            &text,
        );
    }

    // Negative: none of the fatal-failure strings may appear anywhere on
    // screen. `local state store is unavailable` is the #51 signature; the
    // other two guard against a shell takeover or any other fatal line.
    assert_not_in(
        &text,
        "local state store is unavailable",
        &format!("store-unavailable error painted on {label}"),
        &text,
    );
    assert_not_in(
        &text,
        "command not found",
        &format!("shell error on {label}"),
        &text,
    );
    assert_not_in(
        &text,
        "Error:",
        &format!("fatal Error: line on {label}"),
        &text,
    );
    // Negative: the pre-TUI raw stdin prompt must not pre-empt the
    // interface. The pane carries the same words, so this pins the
    // rendering boundary: the question arrives inside the alternate
    // screen (the title above proves it), never as a bare line on the
    // plain terminal before the first paint.
    assert_not_in(
        &text,
        "Trust this folder as the session workspace? [t]rust once",
        &format!("raw line-prompt bytes leaked onto {label}"),
        &text,
    );
}

// ---------------------------------------------------------------------------
// interactive command-path smoke: the dispatch chain on the real TUI
// ---------------------------------------------------------------------------
