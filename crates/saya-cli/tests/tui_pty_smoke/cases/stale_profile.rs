/// A profile whose SQLite file is not there does not fail the launch: the
/// surface opens anyway, as a session with no database, says so once, and a
/// question still gets an answer — the database plugin is not attached at all.
///
/// The launch opens the configured profile's connection before the first frame
/// (`ref/REPORT-T19.md` "Findings" 3), so a file that was deleted underneath the
/// config used to end the process with `Error: SQLite connection failed` and no
/// surface. What an operator needs is the surface, the reason, and a way back
/// (`/connect`, which opens a new run over a profile that is there).
#[test]
fn a_missing_database_file_opens_the_surface_and_says_so() {
    let home = scratch_home("surface-stale-profile");
    let _cleanup = HomeGuard(home.clone());
    let provider = MockProvider::spawn();
    let mut tui = match surface_tui_without_database(&home, Some(&provider)) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping a_missing_database_file_opens_the_surface_and_says_so: {reason}");
            return;
        }
    };
    // The launch opened: the trust question is up, the status line is painted
    // and it names no database — the profile it was configured on is marked
    // unavailable rather than claimed.
    tui.wait_for("trust this folder?", INTERACTIVE)
        .expect("the surface opened with no database attached");
    tui.wait_for("no database", INTERACTIVE)
        .expect("the status line states the absence, not the configured profile");
    tui.dismiss_trust();
    // One line in the transcript names the profile and the error.
    tui.wait_for("cannot open profile 'demo'", INTERACTIVE)
        .expect("the transcript names the profile that could not be opened");
    tui.wait_for("SQLite connection failed", INTERACTIVE)
        .expect("and the error it failed with");
    assert!(
        !tui.screen().contains("Error: SQLite connection failed"),
        "the launch never dies on the connection: {}",
        tui.screen()
    );
    // A question is still answered: the run is a session without a database,
    // never a session that cannot run.
    tui.send_line("what is two plus two?")
        .expect("send the question");
    tui.wait_for(MockProvider::ANSWER, INTERACTIVE)
        .expect("the answer paints in the transcript");
    assert_eq!(
        provider.requests(),
        1,
        "the question reached the provider exactly once"
    );
    assert!(
        tui.still_running(),
        "the surface is alive after the turn: {}",
        tui.screen()
    );
}
