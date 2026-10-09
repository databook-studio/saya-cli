/// The positive/negative assertion set every rung shares: the splash must
/// paint and the active profile must appear in the status bar, while none of
/// the fatal-failure strings may appear anywhere on screen. The negatives are
/// the point — a test that only checks the happy string passes on a screen
/// that also contains `Error: local state store is unavailable`, which is
/// exactly the defect (#51) this packet hunts.
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
/// Every rung launches outside any worktree, so the startup trust modal is
/// open on the settled screen: the splash paints first (the positives
/// below) and the question rides inside the interface (the modal
/// positive), never as a raw stdin read in front of it.
fn assert_splash_for_profile(screen: &vt100::Screen, profile: &str, label: &str) {
    let text = screen_text(screen);

    // Positive: the first screen the REPL paints must be present — asserted on
    // what survives a squeeze, not on the branding.
    //
    // This used to pin `◆ saya` and the tagline. Those are decoration, and the
    // empty state drops decoration before guidance when height is short
    // (Phase 9 packet 2): here the startup trust modal takes most of the pane.
    // It then pinned the concept line; the state section's three readiness
    // lines plus the entry choices tier with the state now, so the concept
    // line yields on this squeezed pane too (U03) — the readiness lines are
    // what the first screen must still say. The offline config is Ollama
    // with model `smoke`, so the AI line reads ready.
    assert_in(
        &text,
        "/ commands",
        &format!("keyboard hint missing from {label} — it must outlive decoration"),
        &text,
    );
    assert_in(
        &text,
        "AI        ready — ollama, model smoke",
        &format!("splash AI readiness line missing from {label}"),
        &text,
    );
    assert_in(
        &text,
        &format!("Database  {profile}"),
        &format!("splash database line missing from {label}"),
        &text,
    );

    // Positive: the status bar names the active profile, followed by the
    // bar's own separator (the splash's database list also names it, so the
    // bare name alone would not prove the bar painted).
    assert_in(
        &text,
        &format!("{profile} · "),
        &format!("status bar profile `{profile} · ` missing from {label}"),
        &text,
    );

    // Positive: the startup trust modal is open — the TUI's rendering of
    // the one trust decision, beside the splash, never before it. The
    // launch dir is outside every worktree with no `--workspace`, so the
    // decision fires; the modal carries the shared prompt's body and all
    // three exits.
    assert_in(
        &text,
        "trust this folder?",
        &format!("trust modal title missing from {label}"),
        &text,
    );
    assert_in(
        &text,
        "Trust this folder as the session",
        &format!("trust modal body missing from {label}"),
        &text,
    );
    for exit in ["[t]rust once", "[w]orkspace", "[c]ontinue unbound"] {
        assert_in(
            &text,
            exit,
            &format!("trust modal exit {exit} missing from {label}"),
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
    // interface. The modal carries the same words, so this pins the
    // rendering boundary: the question arrives inside the alternate
    // screen (the title above proves it), never as a bare line on the
    // plain terminal before the splash paints.
    assert_not_in(
        &text,
        "Trust this folder as the session workspace? [t]rust once",
        &format!("raw line-prompt bytes leaked onto {label}"),
        &text,
    );
}

// ---------------------------------------------------------------------------
// interactive command-path smoke: the dispatch chain on the real TUI (P0)
// ---------------------------------------------------------------------------
