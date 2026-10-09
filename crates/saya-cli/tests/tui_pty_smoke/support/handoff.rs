// /setup: the terminal handoff to the guided setup (U06)
// ---------------------------------------------------------------------------

/// Alternate-screen enter/leave, byte-exact. Mirrored locally from
/// `tui_verbose_metrics_pty.rs`, whose helpers are private to that binary.
const ALT_ENTER: [u8; 8] = [0x1b, b'[', b'?', b'1', b'0', b'4', b'9', b'h'];
/// Alternate-screen leave, byte-exact.
const ALT_LEAVE: [u8; 8] = [0x1b, b'[', b'?', b'1', b'0', b'4', b'9', b'l'];

/// The cursor-position report request crossterm writes when ratatui's
/// `Terminal::clear` snapshots the cursor (a real emulator answers with
/// `ESC[<row>;<col>R`).
const CURSOR_QUERY: &[u8] = b"\x1b[6n";

/// First (leftmost) position of `needle` in `haystack`.
fn position_of(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Last (rightmost) position of `needle` in `haystack`.
fn rposition_of(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// How many times `needle` occurs in `haystack`.
fn count_of(haystack: &[u8], needle: &[u8]) -> usize {
    if haystack.len() < needle.len() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

/// The wizard's first prompt — the provider menu, word for word from
/// `setup/prompt_provider.rs`. Everything the handoff tests wait on hangs
/// off this so a wording drift fails at the first wait, not somewhere deep.
const FIRST_PROMPT: &str = "Choose an AI provider:";

/// Launches the TUI the way a setup user runs it: the configuration lives in
/// the user config dir (`SAYA_CONFIG_HOME/saya`), discovered WITHOUT
/// `--config`/`--connections` flags. This matters for the full-wizard test:
/// the wizard's commit reload check reads the files it wrote through that
/// same discovery, so the new profile resolves — with the flags the reload
/// would be redirected to the harness's config and fail the commit.
fn setup_tui(home: &Path) -> Result<InteractiveTui, String> {
    let user_dir = home.join("config-home").join("saya");
    std::fs::create_dir_all(&user_dir).expect("the user config dir builds");
    std::fs::write(user_dir.join("config.toml"), "[ai]\nmodel = 'smoke'\n")
        .expect("the user config writes");
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("could not allocate pty: {e}"))?;
    let mut cmd = CommandBuilder::new(saya_bin());
    cmd.env("HOME", home);
    cmd.env("SAYA_CONFIG_HOME", home.join("config-home"));
    cmd.env("SAYA_SESSION_DIR", home.join("sessions"));
    cmd.env("SAYA_STATE_DB", home.join("state.sqlite3"));
    cmd.cwd(home.join("launch-dir"));
    std::fs::create_dir_all(home.join("launch-dir")).expect("scratch launch dir builds");
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env_remove("XDG_DATA_HOME");
    cmd.env_remove("APPDATA");
    cmd.env_remove("SAYA_API_KEY");
    cmd.env_remove("SAYA_AI_API_KEY");
    cmd.args(["--approval-mode", "read-only"]);
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("could not spawn saya on pty: {e}"))?;
    drop(pair.slave);
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("could not take pty writer: {e}"))?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("could not clone pty reader: {e}"))?;
    let rx = reader_channel(reader);
    Ok(InteractiveTui {
        writer,
        parser: vt100::Parser::new(ROWS, COLS, 0),
        rx,
        child: ChildGuard::new(child),
        raw: Vec::new(),
        cursor_replies: 0,
    })
}

/// The raw-stream proof of the handoff, on the captured byte stream: the
/// initial alternate-screen enter precedes the handoff's leave, the wizard's
/// first prompt printed BETWEEN the leave and the second enter (the normal
/// screen), the TUI re-entered after it, and enters == leaves + 1 while the
/// child is alive.
fn assert_raw_handoff_ordering(tui: &InteractiveTui) {
    let raw = &tui.raw;
    let enter1 = position_of(raw, &ALT_ENTER).expect("the TUI entered the alternate screen");
    let leave = rposition_of(raw, &ALT_LEAVE).expect("the handoff left the alternate screen");
    let enter2 =
        rposition_of(raw, &ALT_ENTER).expect("the handoff re-entered the alternate screen");
    let wizard =
        position_of(raw, FIRST_PROMPT.as_bytes()).expect("the wizard printed its first prompt");
    assert!(enter1 < leave, "the release came after the initial enter");
    assert!(
        leave < wizard,
        "the wizard printed in the normal screen, not the alternate one"
    );
    assert!(
        wizard < enter2,
        "the TUI re-entered the alternate screen only after the wizard"
    );
    assert_eq!(
        count_of(raw, &ALT_ENTER),
        count_of(raw, &ALT_LEAVE) + 1,
        "enter == leave + 1 while the child is alive"
    );
}
