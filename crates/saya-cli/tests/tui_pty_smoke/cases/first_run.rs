/// The whole first-run path inside one TUI session (U07): the first screen
/// reads `Database  none yet` (an EMPTY config home, no connections file),
/// `/setup` runs the sqlite wizard through the handoff, and back in the TUI
/// the automatic reload applies the written profile — the close block
/// reports it and `/connections` lists `team` from the same refreshed
/// profile list the first screen reads. No restart, no shell command.
///
/// The splash itself cannot repaint after `/setup` in this session: the
/// typed `/setup` line is a user block (`submit.rs`), so the empty state
/// yields to the transcript for good. The refreshed list it would read is
/// proven here through `/connections`, and the splash's own data sources
/// (`app.profiles`, `app.startup`) are proven swapped at unit level
/// (`apply_reloaded_runtime_refreshes_profiles_and_startup_from_the_new_runtime`).
#[test]
fn first_run_setup_applies_immediately_without_a_restart() {
    let home = scratch_home("first-run-setup");
    let _cleanup = HomeGuard(home.clone());
    // An empty file is a valid empty SQLite database, so the probe succeeds.
    let db = home.join("team.db");
    std::fs::write(&db, b"").expect("the empty sqlite file creates");
    // `setup_tui` writes only the user config.toml; connections.toml stays
    // absent, so the first screen reads `none yet`.
    let mut tui = match setup_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping first_run_setup: {reason}");
            return;
        }
    };
    dismiss_trust_modal(&mut tui);
    tui.wait_for("Database  none yet", INTERACTIVE)
        .expect("the first screen reads the absent database");

    tui.send_line("/setup").expect("send /setup");
    // The first Enter is consumed by the completion popup (a bare command's
    // accept), so a second one submits.
    tui.send("\r").expect("submit /setup");
    tui.wait_for(FIRST_PROMPT, INTERACTIVE)
        .expect("the wizard took the terminal");
    tui.send_line("6").expect("skip the provider");
    tui.wait_for("Choose a database:", INTERACTIVE)
        .expect("the database menu");
    tui.send_line("1").expect("sqlite");
    tui.wait_for("Database file path:", INTERACTIVE)
        .expect("the path prompt");
    tui.send_line(db.to_str().expect("utf-8 db path"))
        .expect("send the db path");
    tui.wait_for("Profile name [sqlite]:", INTERACTIVE)
        .expect("the profile name prompt");
    tui.send_line("team").expect("send the profile name");
    tui.wait_for("Write these files?", INTERACTIVE)
        .expect("the confirm prompt after the review and probe");
    tui.send_line("y").expect("confirm the write");

    // Back in the TUI: ONE block carries the close and the reload report.
    tui.wait_for("Guided setup closed.", INTERACTIVE)
        .expect("the close block after the commit");
    tui.wait_for("connections", INTERACTIVE)
        .expect("the reload report names the connections change");

    // The swap is live: the profile list the first screen reads lists the
    // new profile.
    tui.send_line("/connections").expect("send /connections");
    tui.send("\r").expect("submit /connections");
    tui.wait_for("Profiles: team", INTERACTIVE)
        .expect("the profile list follows the swap");
    tui.send_line("/help").expect("send /help");
    tui.send("\r").expect("submit /help");
    tui.wait_for("List runs, or show one run by id", INTERACTIVE)
        .expect("the TUI is still responsive after the applied setup");
    assert!(
        tui.still_running(),
        "the TUI is still alive after the applied setup"
    );
}

/// A cancelled wizard (EOF at the first prompt) changes nothing: the close
/// block says so through the reload's unchanged line, no file was written,
/// and the TUI is responsive. (The splash itself cannot repaint after the
/// typed `/setup` — it is a user block — so `unchanged` here is the reload's
/// own report plus the absent connections file.)
#[test]
fn first_run_setup_cancel_changes_nothing() {
    let home = scratch_home("first-run-cancel");
    let _cleanup = HomeGuard(home.clone());
    let mut tui = match setup_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping first_run_setup_cancel: {reason}");
            return;
        }
    };
    dismiss_trust_modal(&mut tui);
    tui.wait_for("Database  none yet", INTERACTIVE)
        .expect("the first screen before setup");

    tui.send_line("/setup").expect("send /setup");
    tui.send("\r").expect("submit /setup");
    tui.wait_for(FIRST_PROMPT, INTERACTIVE)
        .expect("the wizard took the terminal");
    tui.send("\x04").expect("send EOF");
    tui.wait_for("Guided setup closed.", INTERACTIVE)
        .expect("the close block after the cancel");
    tui.wait_for("Configuration is unchanged.", INTERACTIVE)
        .expect("the automatic reload reports unchanged");
    assert!(
        !home.join("config-home/saya/connections.toml").exists(),
        "nothing was written"
    );
    tui.send_line("/help").expect("send /help");
    tui.send("\r").expect("submit /help");
    tui.wait_for("List runs, or show one run by id", INTERACTIVE)
        .expect("input works again after the cancelled setup");
    assert!(
        tui.still_running(),
        "the TUI is still alive after the cancelled setup"
    );
}

/// A wizard FAILURE returns the TUI to the user with the error block: a
/// pre-seeded `team` profile makes the flow's plan refuse the duplicate name
/// before anything is written — a real, cheap failure (exit 2, no chmod, no
/// cleanup) — and the TUI reports the exit code and stays responsive.
#[test]
fn setup_handoff_failure_returns_to_the_tui() {
    let home = scratch_home("setup-fail");
    let _cleanup = HomeGuard(home.clone());
    let db = home.join("taken.db");
    std::fs::write(&db, b"").expect("the empty sqlite file creates");
    // The user config dir already holds a `team` profile (setup_tui wrote
    // config.toml beside it), so the wizard's plan refuses the name. The
    // dir exists before this write so the seed lands pre-launch.
    std::fs::create_dir_all(home.join("config-home/saya")).expect("the user config dir builds");
    std::fs::write(
        home.join("config-home/saya/connections.toml"),
        format!(
            "[profiles.team]\ntype = 'sqlite'\npath = '{}'\nread_only = false\n",
            db.display()
        ),
    )
    .expect("the seed connections write");
    let mut tui = match setup_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping setup_handoff_failure: {reason}");
            return;
        }
    };
    dismiss_trust_modal(&mut tui);

    tui.send_line("/setup").expect("send /setup");
    // The first Enter is consumed by the completion popup (a bare command's
    // accept), so a second one submits.
    tui.send("\r").expect("submit /setup");
    tui.wait_for(FIRST_PROMPT, INTERACTIVE)
        .expect("the wizard took the terminal");
    tui.send_line("6").expect("skip the provider");
    tui.wait_for("Choose a database:", INTERACTIVE)
        .expect("the database menu");
    tui.send_line("1").expect("sqlite");
    tui.wait_for("Database file path:", INTERACTIVE)
        .expect("the path prompt");
    tui.send_line(db.to_str().expect("utf-8 db path"))
        .expect("send the db path");
    tui.wait_for("Profile name [sqlite]:", INTERACTIVE)
        .expect("the profile name prompt");
    tui.send_line("team").expect("send the profile name");
    tui.wait_for("Guided setup ended with exit code 2", INTERACTIVE)
        .expect("the failure block after the refused plan");
    assert!(
        position_of(&tui.raw, b"already exists in connections.toml").is_some(),
        "the wizard's own refusal printed in the normal screen"
    );
    tui.send_line("/help").expect("send /help");
    tui.send("\r").expect("submit /help");
    tui.wait_for("List runs, or show one run by id", INTERACTIVE)
        .expect("input works again after the failed handoff");
    assert!(
        tui.still_running(),
        "the TUI is still alive after the failed wizard"
    );
}
