/// The full sqlite wizard through the handoff: every prompt answered in turn
/// (the pty cannot be pre-scripted here — typed-ahead bytes before the
/// release are consumed by crossterm, so each prompt is waited for), the
/// profile lands in the user config dir, and the TUI comes back responsive.
#[test]
fn setup_handoff_full_wizard_writes_config_and_returns() {
    let home = scratch_home("setup-full");
    let _cleanup = HomeGuard(home.clone());
    // An empty file is a valid empty SQLite database, so the probe succeeds.
    let db = home.join("team.db");
    std::fs::write(&db, b"").expect("the empty sqlite file creates");
    let mut tui = match setup_tui(&home) {
        Ok(tui) => tui,
        Err(reason) => {
            eprintln!("skipping setup_handoff_full_wizard: {reason}");
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
    tui.wait_for("Write these files?", INTERACTIVE)
        .expect("the confirm prompt after the review and probe");
    tui.send_line("y").expect("confirm the write");
    tui.wait_for("Guided setup closed", INTERACTIVE)
        .expect("the TUI took the terminal back after the commit");
    // U07: the closed wizard's configuration is applied by the automatic
    // reload — the same one /reload runs — so the report names the new
    // connections instead of deferring to the next session.
    tui.wait_for("connections", INTERACTIVE)
        .expect("the reload report names the connections change");

    let written = std::fs::read_to_string(home.join("config-home/saya/connections.toml"))
        .expect("the wizard wrote connections.toml");
    assert!(
        written.contains("team"),
        "the profile name landed: {written}"
    );
    assert!(
        written.contains("team.db"),
        "the sqlite path landed: {written}"
    );
    tui.send_line("/help").expect("send /help");
    tui.send("\r").expect("submit /help");
    tui.wait_for("List runs, or show one run by id", INTERACTIVE)
        .expect("input works again after the handoff");
    assert!(
        tui.still_running(),
        "the TUI is still alive after the full wizard"
    );
}
