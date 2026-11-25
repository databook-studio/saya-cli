// The child process and the offline files it reads: a fresh `saya` on a
// scratch HOME, every real config/session/state location redirected under it.

/// The child environment every launch shares: offline, with a real terminal
/// name and no provider key. The launch directory is the child's temporary
/// directory too, so it is a dangerous root and the startup trust question
/// opens — the surface paints first and the question rides inside it.
fn tui_command(home: &Path, config: &Path, connections: &Path) -> CommandBuilder {
    let launch = home.join("launch-dir");
    std::fs::create_dir_all(&launch).expect("scratch launch dir builds");
    let mut cmd = CommandBuilder::new(saya_bin());
    cmd.env("HOME", home);
    // Windows resolves the home directory from USERPROFILE, not HOME; set both
    // so no lookup can escape the scratch tree.
    cmd.env("USERPROFILE", home);
    cmd.env("SAYA_CONFIG_HOME", home.join("config-home"));
    cmd.env("SAYA_SESSION_DIR", home.join("sessions"));
    cmd.env("SAYA_STATE_DB", home.join("state.sqlite3"));
    cmd.env("TMPDIR", &launch);
    cmd.env("TMP", &launch);
    cmd.env("TEMP", &launch);
    // A real terminal name, so the Windows ANSI fallback is deterministic even
    // where the console reports VT support late; harmless on Unix.
    cmd.env("TERM", "xterm-256color");
    cmd.cwd(&launch);
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env_remove("XDG_DATA_HOME");
    cmd.env_remove("APPDATA");
    cmd.env_remove("LOCALAPPDATA");
    cmd.env_remove("SAYA_API_KEY");
    cmd.env_remove("SAYA_AI_API_KEY");
    cmd.args([
        "--config",
        config.to_str().unwrap(),
        "--connections",
        connections.to_str().unwrap(),
        "--approval-mode",
        "read-only",
    ]);
    cmd
}

/// A minimal, offline config + connections file under `home`. `theme = 'dark'`
/// skips the startup OSC background query: it would spend the first 150 ms of
/// a test waiting for a terminal reply that a pty harness never sends, and it
/// is not what this smoke checks. The profile points at an empty SQLite file
/// the run never queries; the file must exist because the launch opens the
/// configured profile.
fn write_config(home: &Path) -> (PathBuf, PathBuf) {
    let config = home.join("config.toml");
    std::fs::write(
        &config,
        "[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n\n[ui]\ntheme = 'dark'\n",
    )
    .unwrap();

    let database = home.join("demo.sqlite3");
    std::fs::write(&database, b"").expect("the demo db file is created");
    let connections = home.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.demo]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();
    (config, connections)
}
