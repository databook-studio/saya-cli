#[tokio::test]
async fn unknown_subcommand_is_a_parse_error() {
    let parsed = parse_slash_command("/investigation bogus");
    assert!(
        parsed.is_err(),
        "an unknown subcommand must be a usage error, got {parsed:?}"
    );
    let message = parsed.unwrap_err().to_string();
    assert!(
        message.contains("usage"),
        "the usage error carries guidance: {message}"
    );
}

// ---------------------------------------------------------------------------
// 6. The headless REPL has no last query: a save with neither --sql nor
//    --file refuses there (S9 invariant 3), never falling through to the
//    operation's stdin read — stdin is the REPL's own input.
// ---------------------------------------------------------------------------
#[test]
fn headless_save_without_sql_refuses_instead_of_reading_stdin() {
    let root = temp_root("headless_save_refusal");
    let investigations = root.join("investigations");
    let config = root.join("config.toml");
    fs::write(&config, "[run]\nmax_rows = 1\n").unwrap();
    let database = root.join("data.sqlite3");
    fs::write(&database, b"").unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();

    // One verbatim turn via --turn-file: the same per-turn entry the piped
    // loop uses, with an exit code that reflects the turn's outcome (an
    // errored turn exits 5).
    let turn = root.join("turn.txt");
    fs::write(&turn, "/investigation save Total orders\n").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--format",
            "json",
            "--turn-file",
            turn.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--connections",
            connections.to_str().unwrap(),
        ])
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("SAYA_INVESTIGATIONS_DIR", &investigations)
        .env("SAYA_CONFIG_HOME", root.join("config-home"))
        .env("HOME", root.join("home"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        output.status.code(),
        Some(5),
        "the refusal must end the turn errored: {stdout}{stderr}"
    );
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("--sql") && combined.contains("--file"),
        "the refusal names both remedies: {combined}"
    );
    // Nothing was saved.
    assert!(
        fs::read_dir(&investigations)
            .map(|entries| entries.count())
            .unwrap_or(0)
            == 0,
        "no document was written: {combined}"
    );

    let _ = fs::remove_dir_all(root);
}
