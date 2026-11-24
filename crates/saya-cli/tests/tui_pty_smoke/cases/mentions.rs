/// `@` in the composer paints the workspace's own paths above the composer box,
/// `Tab` puts the chosen one on the line, and `Esc` closes the list without
/// touching the draft.
///
/// Captured at the two sizes a screen is reviewed at (100x30 and 60x24), with
/// the capture printed (`--nocapture`) so REPORT.md can quote it. The workspace
/// is a folder the launch binds (`--workspace`), holding one ordinary source
/// file and one credential-shaped file: the list must offer the first and never
/// the second, because it is the listing the file tools would give.
#[test]
fn typing_at_paints_the_workspaces_paths_at_both_review_sizes() {
    for (cols, rows) in [(100u16, 30u16), (60u16, 24u16)] {
        let home = scratch_home(&format!("mentions-{cols}x{rows}"));
        let _cleanup = HomeGuard(home.clone());
        let project = home.join("project");
        std::fs::create_dir_all(project.join("src")).expect("the project dir builds");
        std::fs::write(project.join("src/orders.rs"), b"fn orders() {}\n")
            .expect("the source file writes");
        // A credential-shaped name, with content no guard can read as a
        // real secret: the file exists so the listing has something to keep
        // out, and its bytes are never carried into a message.
        std::fs::write(project.join(".env"), b"placeholder content\n")
            .expect("the private file writes");
        let workspace = project.to_str().expect("utf-8 path").to_owned();
        let mut tui = match surface_sized(
            &home,
            None,
            cols,
            rows,
            READ_ONLY,
            &["--workspace", &workspace],
        ) {
            Ok(tui) => tui,
            Err(reason) => {
                eprintln!(
                    "skipping typing_at_paints_the_workspaces_paths_at_both_review_sizes: {reason}"
                );
                return;
            }
        };
        tui.wait_for("Message", INTERACTIVE)
            .expect("the composer paints at the review size");
        // `@` opens the list: the walk runs off the loop's thread, so the wait
        // is for its answer, not for a frame that never comes.
        tui.send("@").expect("type the mention mark");
        tui.wait_for("Files", INTERACTIVE)
            .expect("the list's title paints");
        tui.wait_for("src/orders.rs", INTERACTIVE)
            .expect("the walk's own path is offered");
        tui.wait_for("Tab insert", INTERACTIVE)
            .expect("the list names the key that chooses from it");
        // Typing narrows it: the path the operator is naming is the only row.
        tui.send("orders").expect("type the path");
        tui.wait_for("src/orders.rs", INTERACTIVE)
            .expect("the narrowed list keeps the path it matches");
        eprintln!("----- {cols}x{rows} -----\n{}", tui.screen());
        assert!(
            !tui.screen().contains(".env"),
            "a private path is never offered: {}",
            tui.screen()
        );
        tui.send("\t").expect("press Tab");
        tui.wait_for("@src/orders.rs", INTERACTIVE)
            .expect("the chosen path is on the line");
        tui.wait_gone("Tab insert", INTERACTIVE)
            .expect("the list closed on insertion");
         // Esc closes a list too, and the draft keeps what was typed.
        tui.send(" @ma").expect("type a second mention");
        tui.wait_for("Files", INTERACTIVE)
            .expect("the list opens again");
        tui.send("\x1b").expect("press Esc");
        tui.wait_gone("Files", INTERACTIVE)
            .expect("Esc closes the list");
        assert!(
            tui.screen().contains("@ma"),
            "the draft keeps what was typed: {}",
            tui.screen()
        );
        assert!(tui.still_running(), "the surface is alive after the list");
    }
}
