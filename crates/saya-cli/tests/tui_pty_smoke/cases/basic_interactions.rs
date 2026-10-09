/// `/sql` then `/export --snapshot` on the real TUI: the file lands and the
/// TUI survives. The P0 dispatch panic killed the session on exactly this
/// path (`helper passes the action through` at the chain's next step).
#[test]
fn tui_sql_then_export_snapshot_survives_and_writes_the_file() {
    let home = scratch_home("sql-export");
    let _cleanup = HomeGuard(home.clone());
    let mut tui = match interactive_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping tui_sql_then_export_snapshot: {reason}");
            return;
        }
    };
    dismiss_trust_modal(&mut tui);

    tui.send_line("/sql SELECT 1").expect("send /sql");
    tui.wait_for("direct sql", INTERACTIVE)
        .expect("the query result paints");

    let snapshot = home.join("snapshot.csv");
    tui.send_line(&format!("/export --snapshot {}", snapshot.display()))
        .expect("send /export");
    tui.wait_for("Exported", INTERACTIVE)
        .expect("the export is said");
    assert!(
        snapshot.exists(),
        "the snapshot file was written by the TUI"
    );
    assert!(
        tui.still_running(),
        "the TUI is still alive after the export"
    );
}

/// `/sql` then `/investigation save <name>` on the real TUI: the save fills
/// from the last query, stores the document, and the TUI survives.
#[test]
fn tui_sql_then_investigation_save_survives_and_saves() {
    let home = scratch_home("sql-save");
    let _cleanup = HomeGuard(home.clone());
    let mut tui = match interactive_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping tui_sql_then_investigation_save: {reason}");
            return;
        }
    };
    dismiss_trust_modal(&mut tui);

    tui.send_line("/sql SELECT 1").expect("send /sql");
    tui.wait_for("direct sql", INTERACTIVE)
        .expect("the query result paints");

    tui.send_line("/investigation save pty-sentinel")
        .expect("send /investigation save");
    tui.wait_for("Saved exactly as shown", INTERACTIVE)
        .expect("the save is said");
    assert!(tui.still_running(), "the TUI is still alive after the save");
}

// ---------------------------------------------------------------------------
// /reload on the real TUI: apply a changed config between turns (O04)
// ---------------------------------------------------------------------------

/// `/reload` on the real TUI: unchanged files say so, an edited `--config`
/// file swaps the runtime and names the key, a broken file says the previous
/// configuration is still in use — and the TUI stays responsive through all
/// of it. The edited file is the same `home/config.toml` `interactive_tui`
/// passed as `--config`, written while the child is running.
#[test]
fn tui_reload_applies_edits_and_survives_a_broken_file() {
    let home = scratch_home("reload");
    let _cleanup = HomeGuard(home.clone());
    let config = home.join("config.toml");
    let mut tui = match interactive_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping tui_reload: {reason}");
            return;
        }
    };
    dismiss_trust_modal(&mut tui);

    // Unchanged: the pinned unchanged line.
    tui.send_line("/reload").expect("send /reload");
    tui.send("\r").expect("submit /reload");
    tui.wait_for("Configuration is unchanged", INTERACTIVE)
        .expect("the unchanged reload paints");

    // Edited: the plan names the key and the swap is live — a following
    // unchanged reload would say so, but the plan text is the evidence.
    std::fs::write(&config, "[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 5\n")
        .expect("the edited config writes");
    tui.send_line("/reload")
        .expect("send /reload after the edit");
    tui.send("\r").expect("submit /reload");
    tui.wait_for("max_rows: 10 -> 5", INTERACTIVE)
        .expect("the edited reload names the change");

    // Broken: the pinned failure sentence, and the TUI still answers.
    std::fs::write(&config, "[run\n").expect("the broken config writes");
    tui.send_line("/reload")
        .expect("send /reload over the broken file");
    tui.send("\r").expect("submit /reload");
    tui.wait_for("Reload failed", INTERACTIVE)
        .expect("the broken file fails");
    tui.wait_for("previous configuration is still in use", INTERACTIVE)
        .expect("the pinned sentence paints");
    tui.send_line("/help").expect("send /help");
    tui.send("\r").expect("submit /help");
    tui.wait_for("List runs, or show one run by id", INTERACTIVE)
        .expect("the TUI is still responsive after the failure");
    assert!(
        tui.still_running(),
        "the TUI is still alive after the failed reload"
    );
    // Any key dismisses the help overlay. The restore leg below doubles as
    // the dismissal proof: with the overlay open, its first keystroke would
    // close the overlay instead of typing `/reload`, and the expected plan
    // line would never paint.
    tui.send("\x1b").expect("dismiss the help overlay");

    // Restored: the reload works again, from the failed value to the
    // restored one.
    std::fs::write(&config, "[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n")
        .expect("the restored config writes");
    tui.send_line("/reload")
        .expect("send /reload after restore");
    tui.send("\r").expect("submit /reload");
    tui.wait_for("max_rows: 5 -> 10", INTERACTIVE)
        .expect("the restored reload works again");
    assert!(tui.still_running(), "the TUI is still alive at the end");
}

// ---------------------------------------------------------------------------
// /investigation run --param never persists or echoes (D2, audit A922-2)
// ---------------------------------------------------------------------------
