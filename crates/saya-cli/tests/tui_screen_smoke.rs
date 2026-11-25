//! Smoke test that the full-screen interface actually paints, from a real
//! process, on every platform we ship.
//!
//! The sibling `tui_pty_smoke` suite is Unix-only: its cases are written
//! against Unix terminal modes. This file exists for the one question that
//! suite cannot answer — whether the default full-screen interface paints
//! under ConPTY on Windows — and it asks it the same way on macOS, Linux and
//! Windows: launch the real binary on a native pseudo-terminal, read the
//! screen back, and assert the words the interface promises.
//!
//! The sequence is the first minute of a real session: the welcome screen,
//! status row and composer paint; the startup trust question rides inside the
//! interface; a slash opens the command palette; Esc closes it; `/exit`
//! leaves. A green exit code proves nothing on its own — the process can exit
//! at startup with a fatal line and never paint — so every step asserts text.

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

include!("support/provider_env.rs");
include!("tui_screen_smoke/support/answers.rs");
include!("tui_screen_smoke/support/pty.rs");
include!("tui_screen_smoke/support/spawn.rs");
include!("tui_screen_smoke/support/command.rs");
include!("tui_screen_smoke/support/guards.rs");

/// The whole launch-to-exit path, asserted on the screen text. The needles are
/// the interface's own words, read from the modules that paint them:
/// `view::empty_lines` for the unbound welcome, `view::status` for `? help` and
/// the `mode │ model │ database` row, `view::composer` for the box and its
/// hint, `view::trust` for the startup question, and `view::palette` for the
/// command palette.
#[test]
fn full_screen_ui_draws_and_answers_slash_commands() {
    let home = scratch_home("draws");
    // Clean up the scratch HOME however this test ends.
    let _cleanup = HomeGuard(home.clone());

    // A pty that cannot be allocated or a child that cannot be spawned is a
    // failure of this check, not a reason to skip it: a green run has to mean
    // the interface was actually painted.
    let mut tui = launch(&home).expect("launch saya on a pty");

    // 1. The welcome screen paints: the session header names the product and
    //    says no workspace is bound, and the one line explains what is off.
    tui.wait_for("Saya", INTERACTIVE)
        .expect("the welcome header paints the product name");
    tui.wait_for("no workspace (outside a git repository)", INTERACTIVE)
        .expect("the welcome header names the unbound workspace");
    tui.wait_for("File tools are off here.", INTERACTIVE)
        .expect("the welcome line says what the unbound session cannot do");

    // 2. The status row paints end to end: the idle word, the mode chip, the
    //    configured model and the active profile, in one row.
    tui.wait_for("? help", INTERACTIVE)
        .expect("the status row paints its idle word");
    tui.wait_for("read-only │ smoke │ demo", INTERACTIVE)
        .expect("the status row names mode, model and database");

    // 3. The composer paints its box and its hint row.
    tui.wait_for("Message", INTERACTIVE)
        .expect("the composer box paints");
    tui.wait_for("Enter send", INTERACTIVE)
        .expect("the composer hint row paints");

    // 4. The startup trust question rides the interface, never a raw stdin
    //    read before the first paint. Answering `n` continues unbound.
    tui.wait_for("trust this folder?", INTERACTIVE)
        .expect("the trust question rides the interface");
    tui.send("n").expect("answer the trust question");
    tui.wait_gone("trust this folder?", INTERACTIVE)
        .expect("the trust pane closes after its answer");
    assert!(
        tui.still_running(),
        "the surface is alive after its first frame"
    );

    // 5. A slash opens the command palette: its title is on screen.
    tui.send("/").expect("open the command palette");
    tui.wait_for("Commands", INTERACTIVE)
        .expect("the command palette paints");

    // 6. Esc closes the palette and gives the composer back.
    tui.send("\x1b").expect("press Esc");
    tui.wait_gone("Commands", INTERACTIVE)
        .expect("Esc closes the command palette");
    tui.wait_for("Message", INTERACTIVE)
        .expect("the composer is back after Esc");
    assert!(tui.still_running(), "the surface is alive after Esc");

    // 7. `/exit` leaves the surface on its own.
    tui.send_line("/exit").expect("send /exit");
    tui.wait_exit(INTERACTIVE)
        .expect("the surface left on its own after /exit");
}
