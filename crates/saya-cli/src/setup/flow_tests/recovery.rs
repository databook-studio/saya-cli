#[test]
fn interrupted_marker_restore_path_restores_originals() {
    let dir = temp_dir("rec-restore");
    let original = "[profiles.a]\ntype = \"sqlite\"\npath = \"/tmp/a.db\"\nread_only = true\n";
    let rewritten = format!(
        "{original}\n[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n"
    );
    fs::write(dir.join("connections.toml"), original).unwrap();
    stage_interrupted(&dir, original, &rewritten);

    // r = restore, then skip the provider (6) and the database (8).
    let (code, out) = run_flow(
        &dir,
        "r\n6\n8\n",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fs::read_to_string(dir.join("connections.toml")).unwrap(),
        original,
        "restore put the original bytes back"
    );
    assert!(!dir.join(MARKER_FILE).exists(), "the marker is cleared");
    assert!(!dir.join(BACKUP_DIR).exists(), "the backups are cleared");
    assert!(
        out.contains("Restored the previous files"),
        "the flow says the restore happened: {out}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn interrupted_marker_finish_keeps_new_bytes() {
    let dir = temp_dir("rec-finish");
    let original = "[profiles.a]\ntype = \"sqlite\"\npath = \"/tmp/a.db\"\nread_only = true\n";
    let rewritten = format!(
        "{original}\n[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n"
    );
    fs::write(dir.join("connections.toml"), original).unwrap();
    stage_interrupted(&dir, original, &rewritten);

    let (code, out) = run_flow(
        &dir,
        "f\n6\n8\n",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fs::read_to_string(dir.join("connections.toml")).unwrap(),
        rewritten,
        "finish keeps the current files"
    );
    assert!(!dir.join(MARKER_FILE).exists(), "the marker is cleared");
    assert!(out.contains("Kept the current files"), "{out}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn interrupted_marker_quit_changes_nothing() {
    let dir = temp_dir("rec-quit");
    let original = "[profiles.a]\ntype = \"sqlite\"\npath = \"/tmp/a.db\"\nread_only = true\n";
    let rewritten = format!(
        "{original}\n[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n"
    );
    fs::write(dir.join("connections.toml"), original).unwrap();
    stage_interrupted(&dir, original, &rewritten);

    let (code, out) = run_flow(
        &dir,
        "q\n",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fs::read_to_string(dir.join("connections.toml")).unwrap(),
        rewritten,
        "quit neither restores nor writes"
    );
    assert!(dir.join(MARKER_FILE).exists(), "quit leaves the marker");
    let _ = fs::remove_dir_all(dir);
}

/// A restore that does not complete never prints the success text: the flow
/// prints each failure and the marker-kept guidance, then fails (exit 2 via
/// the propagated error), leaving the pending state for a fixed retry.
#[test]
fn restore_failure_prints_failures_and_never_the_success_text() {
    let dir = temp_dir("rec-restore-fail");
    let original = "[profiles.a]\ntype = \"sqlite\"\npath = \"/tmp/a.db\"\nread_only = true\n";
    fs::write(dir.join("connections.toml"), original).unwrap();
    // Two-entry marker (mirrors the engine tests): connections.toml with a
    // present backup, and the created config.toml — replaced by a directory,
    // so removing it fails.
    let marker = serde_json::json!({
        "version": 1,
        "started_unix_ms": 42,
        "entries": [
            { "file": "connections.toml", "backup": "connections.toml", "created": false },
            { "file": "config.toml", "backup": null, "created": true }
        ]
    });
    fs::write(dir.join(MARKER_FILE), serde_json::to_vec(&marker).unwrap()).unwrap();
    fs::create_dir_all(dir.join(BACKUP_DIR)).unwrap();
    fs::write(dir.join(BACKUP_DIR).join("connections.toml"), original).unwrap();
    fs::create_dir_all(dir.join("config.toml")).unwrap();

    let mut opts = options(&dir);
    let mut input = Cursor::new("r\n");
    let mut output = Vec::new();
    let result = run_with(&mut opts, &mut input, &mut output);
    let error = result.expect_err("an incomplete restore is an error");
    assert!(
        error.to_string().contains("restore did not complete"),
        "the error says the restore did not complete: {error}"
    );
    let out = String::from_utf8(output).unwrap();
    assert!(
        !out.contains("Restored the previous files"),
        "the success text is never printed on failure: {out}"
    );
    assert!(
        out.contains("The recovery marker was kept; fix the file and run `saya setup` again."),
        "the marker-kept guidance is printed: {out}"
    );
    assert!(
        out.contains("config.toml"),
        "each failure names its file: {out}"
    );
    assert!(dir.join(MARKER_FILE).exists(), "the marker is kept");
    assert!(dir.join(BACKUP_DIR).exists(), "the backups are kept");
    assert!(
        !out.contains("No files were changed."),
        "the flow stops; no false cancel line: {out}"
    );
    let _ = fs::remove_dir_all(dir);
}

/// A recovery marker that cannot be read stops the flow at once, naming the
/// marker path and the manual way out: `commit` would refuse the pending
/// state only at the very end, after the whole flow has been answered. The
/// marker is never deleted automatically.
#[test]
fn unreadable_marker_stops_the_flow_naming_the_way_out() {
    let dir = temp_dir("rec-unreadable");
    let marker_path = dir.join(MARKER_FILE);
    fs::write(&marker_path, b"{ interrupted, but corrupt").unwrap();

    let mut opts = options(&dir);
    // A full script: if the flow continued, it would run to the commit that
    // refuses at the very end.
    let mut input = Cursor::new(SQLITE_SCRIPT);
    let mut output = Vec::new();
    let result = run_with(&mut opts, &mut input, &mut output);

    let error = result.expect_err("an unreadable marker stops the flow");
    let message = error.to_string();
    let marker = marker_path.to_str().unwrap();
    assert!(
        message.contains(marker),
        "the message names the marker path: {message}"
    );
    assert!(
        message.contains("inspect or remove") && message.contains("then run `saya setup` again"),
        "the message names the manual way out: {message}"
    );
    let out = String::from_utf8(output).unwrap();
    assert!(
        !out.contains("database") && !out.contains("Nothing to configure"),
        "the flow stopped before any prompt: {out}"
    );
    assert!(
        marker_path.exists(),
        "the marker is never deleted automatically"
    );
    assert!(
        !dir.join("connections.toml").exists() && !dir.join("config.toml").exists(),
        "nothing was written"
    );
    let _ = fs::remove_dir_all(dir);
}
