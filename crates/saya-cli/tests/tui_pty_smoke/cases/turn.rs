/// A typed question runs one turn against the provider and the answer paints:
/// the session the launch composed is the one the surface talks to.
#[test]
fn a_typed_question_runs_a_turn_and_the_answer_paints() {
    let home = scratch_home("surface-turn");
    let _cleanup = HomeGuard(home.clone());
    let provider = MockProvider::spawn();
    let mut tui = match surface_tui(&home, Some(&provider)) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping a_typed_question_runs_a_turn: {reason}");
            return;
        }
    };
    tui.dismiss_trust();
    tui.send_line("what is two plus two?")
        .expect("send the question");
    tui.wait_for(MockProvider::ANSWER, INTERACTIVE)
        .expect("the answer paints in the transcript");
    assert_eq!(
        provider.requests(),
        1,
        "the question reached the provider exactly once"
    );
    assert!(tui.still_running(), "the surface is alive after the turn");
}

/// Esc during a turn stops it: the turn ends as the operator's cancel, and the
/// run stays open (`02` §3.8).
#[test]
fn esc_during_a_turn_stops_it() {
    let home = scratch_home("surface-esc");
    let _cleanup = HomeGuard(home.clone());
    let provider = MockProvider::spawn();
    let mut tui = match surface_tui(&home, Some(&provider)) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping esc_during_a_turn_stops_it: {reason}");
            return;
        }
    };
    tui.dismiss_trust();
    tui.send_line("what is two plus two?")
        .expect("send the question");
    tui.wait_for("working", INTERACTIVE)
        .expect("the status line says the turn is running");
    tui.send("\x1b").expect("press Esc");
    tui.wait_for("Stopped by you", INTERACTIVE)
        .expect("the cancel paints in the transcript");
    assert!(
        tui.still_running(),
        "the run stays open after the cancel: Esc stops the turn, never the session"
    );
}

/// `/mode` opens the picker: one pane, six modes, and the mode in force marked
/// (`02` §8).
#[test]
fn mode_opens_the_picker() {
    let home = scratch_home("surface-mode");
    let _cleanup = HomeGuard(home.clone());
    let mut tui = match surface_tui(&home, None) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping mode_opens_the_picker: {reason}");
            return;
        }
    };
    tui.dismiss_trust();
    tui.send_line("/mode").expect("send /mode");
    tui.wait_for("accept-edits", INTERACTIVE)
        .expect("the picker's rows paint");
    tui.wait_for("Enter set", INTERACTIVE)
        .expect("the picker's key row paints");
    // Esc closes it without stating anything: the composer is back.
    tui.send("\x1b").expect("press Esc");
    tui.wait_for("Message", INTERACTIVE)
        .expect("the composer is back after the picker closed");
    assert!(tui.still_running(), "the surface is alive after the picker");
}

/// Ctrl+J opens the jobs drawer: the pane the release gives a job list
/// (`02` §8), empty in a session that started none.
#[test]
fn ctrl_j_opens_the_jobs_drawer() {
    let home = scratch_home("surface-jobs");
    let _cleanup = HomeGuard(home.clone());
    let mut tui = match surface_tui(&home, None) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping ctrl_j_opens_the_jobs_drawer: {reason}");
            return;
        }
    };
    tui.dismiss_trust();
    tui.send("\x0a").expect("press Ctrl+J");
    tui.wait_for("Background jobs", INTERACTIVE)
        .expect("the drawer's title paints");
    tui.wait_for("no jobs", INTERACTIVE)
        .expect("the empty drawer says so");
    tui.send("\x1b").expect("press Esc");
    tui.wait_for("Message", INTERACTIVE)
        .expect("the composer is back after the drawer closed");
    assert!(tui.still_running(), "the surface is alive after the drawer");
}
