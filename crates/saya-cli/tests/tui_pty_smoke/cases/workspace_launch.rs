/// The launch screen, captured at the two sizes a screen is reviewed at.
///
/// The workspace summary this job adds is model-facing — it rides the first
/// request's untrusted lane and paints nothing — so the one screen the launch
/// shows must be the screen it showed before: the status row, the composer and
/// the startup trust question, at 100x30 and at 60x24. Both captures are
/// printed (`--nocapture`) so REPORT.md can quote them beside the screen the
/// commit before this one painted.
///
/// The same launch proves the prompt is not held: the composer paints and takes
/// a typed line before any workspace was bound, so the scan a session starts
/// when it binds one cannot be in front of the operator's first keystroke.
#[test]
fn the_launch_paints_the_same_screen_at_both_review_sizes() {
    for (cols, rows) in [(100u16, 30u16), (60u16, 24u16)] {
        let home = scratch_home(&format!("review-{cols}x{rows}"));
        let _cleanup = HomeGuard(home.clone());
        let mut tui = match surface_over_at(&home, None, true, (cols, rows)) {
            Ok(tui) => tui,
            Err(reason) => {
                eprintln!("skipping the_launch_paints_the_same_screen_at_both_review_sizes: {reason}");
                return;
            }
        };
        tui.wait_for("Message", INTERACTIVE)
            .expect("the composer paints at the review size");
        tui.wait_for("Enter send", INTERACTIVE)
            .expect("the composer's hint row paints");
        tui.wait_for("trust this folder?", INTERACTIVE)
            .expect("the trust question rides the interface");
        // The capture is the first-paint screen: the one an operator reads
        // before answering anything.
        eprintln!("----- {cols}x{rows} -----\n{}", tui.screen());
        // The prompt takes input while nothing is bound and no scan has run:
        // answering the question binds the folder, and everything after it is
        // the session's, never a wait for a listing.
        tui.dismiss_trust();
        tui.wait_for("Message", INTERACTIVE)
            .expect("the composer is back after the trust answer");
        tui.send("hi").expect("type into the composer");
        tui.wait_for("hi", INTERACTIVE)
            .expect("the composer takes a typed line at the review size");
        assert!(
            tui.still_running(),
            "the surface is alive after its first frame at {cols}x{rows}"
        );
    }
}

/// A launch over a folder of ten thousand entries: binding it starts the
/// workspace scan after the first paint, and the composer keeps taking input
/// while the scan runs — the bound is one listing, never a walk of the tree.
#[test]
fn a_launch_over_a_huge_folder_still_takes_a_typed_line() {
    let home = scratch_home("review-huge");
    let _cleanup = HomeGuard(home.clone());
    // The launch directory the harness binds: ten thousand files, the bound
    // the summary's own tests state.
    let launch = home.join("launch-dir");
    std::fs::create_dir_all(&launch).expect("the launch dir builds");
    for index in 0..10_000 {
        std::fs::write(launch.join(format!("f{index:05}.txt")), b"").expect("the entry writes");
    }
    let mut tui = match surface_over_at(&home, None, true, (100, 30)) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping a_launch_over_a_huge_folder_still_takes_a_typed_line: {reason}");
            return;
        }
    };
    tui.wait_for("trust this folder?", INTERACTIVE)
        .expect("the trust question is up");
    // Binding the folder is what opens a run over it, and the run's open is
    // where the launch scan starts: the composer must take a line while that
    // scan runs, with ten thousand entries to list.
    tui.bind_trust();
    tui.wait_for("Message", INTERACTIVE)
        .expect("the composer paints over a folder of ten thousand entries");
    tui.send("hi").expect("type into the composer");
    tui.wait_for("hi", INTERACTIVE)
        .expect("the composer takes a typed line while the scan runs");
    assert!(tui.still_running(), "the surface is alive over a huge folder");
}
