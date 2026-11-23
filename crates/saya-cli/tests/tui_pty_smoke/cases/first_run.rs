/// The whole first-run path inside one TUI session: the status row reads
/// `no database` (an EMPTY config home, no connections file), `/setup` runs the
/// sqlite wizard through the handoff, and back in the surface the automatic
/// reload applies the written profile — the close block reports it, and
/// `/connections` lists `team` from the configuration the reload put in force.
/// No restart, no shell command.
///
/// The status row itself does not repaint after `/setup`: a direct command's
/// words are the surface's transient notice, so the refreshed list is proven
/// here through `/connections` — which reads the configuration in force, not a
/// copy taken at launch — and the swap itself at unit level
/// (`front::tui::reload_tests`).
#[test]
fn first_run_setup_applies_immediately_without_a_restart() {
    let home = scratch_home("first-run-setup");
    let _cleanup = HomeGuard(home.clone());
    // An empty file is a valid empty SQLite database, so the probe succeeds.
    let db = home.join("team.db");
    std::fs::write(&db, b"").expect("the empty sqlite file creates");
    // `setup_tui` writes only the user config.toml; connections.toml stays
    // absent, so the status row reads `no database`.
    let mut tui = match setup_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping first_run_setup: {reason}");
            return;
        }
    };
    dismiss_trust_modal(&mut tui);
    // The new surface has one `│`-separated status row and no `Database`
    // column: an absent database is the `no database` segment of that row, which
    // is this surface's word for the old splash's `Database  none yet`.
    tui.wait_for("no database", INTERACTIVE)
        .expect("the first screen reads the absent database");

    tui.send_line("/setup").expect("send /setup");
    // `/` opens the palette, the typed letters narrow it to `/setup`, and the
    // first Enter runs the choice — which submits the line it filled. The
    // second Enter is typed-ahead: the handoff drops what was queued before it
    // released the terminal.
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
    // The proof that input works again is the sheet bare `/help` opens: the
    // new surface answers `/help` with its key sheet (`view::help_keys`), not
    // with the catalogue listing, so the row this pins is the sheet's own.
    tui.wait_for("Conversation", INTERACTIVE)
        .expect("the TUI is still responsive after the applied setup");
    assert!(
        tui.still_running(),
        "the TUI is still alive after the applied setup"
    );
}

/// A cancelled wizard (EOF at the first prompt) changes nothing: the close
/// block says so through the reload's unchanged line, no file was written,
/// and the surface is responsive. `unchanged` is the reload's own report plus
/// the absent connections file.
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
    tui.wait_for("no database", INTERACTIVE)
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
    // The proof that input works again is the sheet bare `/help` opens: the
    // new surface answers `/help` with its key sheet (`view::help_keys`), not
    // with the catalogue listing, so the row this pins is the sheet's own.
    tui.wait_for("Conversation", INTERACTIVE)
        .expect("input works again after the cancelled setup");
    assert!(
        tui.still_running(),
        "the TUI is still alive after the cancelled setup"
    );
}

/// A wizard FAILURE returns the TUI to the user with the error block: a
/// pre-seeded `team` profile makes the flow's plan refuse the duplicate name
/// before anything is written — a real, cheap failure (exit 2, no chmod, no
/// cleanup) — and the surface reports the exit code and stays responsive.
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
    // `/` opens the palette, the typed letters narrow it to `/setup`, and the
    // first Enter runs the choice — which submits the line it filled. The
    // second Enter is typed-ahead: the handoff drops what was queued before it
    // released the terminal.
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
    // The proof that input works again is the sheet bare `/help` opens: the
    // new surface answers `/help` with its key sheet (`view::help_keys`), not
    // with the catalogue listing, so the row this pins is the sheet's own.
    tui.wait_for("Conversation", INTERACTIVE)
        .expect("input works again after the failed handoff");
    assert!(
        tui.still_running(),
        "the TUI is still alive after the failed wizard"
    );
}
