#[test]
fn full_sqlite_flow_writes_connections_and_reloads() {
    let dir = temp_dir("full-sqlite");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (code, out) = run_flow(&dir, SQLITE_SCRIPT, instant_probes(&calls, true, true));
    assert_eq!(code, 0, "the flow succeeds: {out}");
    let connections = dir.join("connections.toml");
    assert!(connections.exists(), "connections.toml is written: {out}");
    let parsed = ConnectionsFile::from_toml(&fs::read_to_string(&connections).unwrap()).unwrap();
    assert_eq!(
        parsed.profiles.get("team"),
        Some(&DatabaseProfile::Sqlite {
            path: "/tmp/saya-flow-test.db".into(),
            read_only: true,
        }),
        "the draft profile round-trips through the written file"
    );
    assert_eq!(
        calls.lock().unwrap().clone(),
        vec!["database:sqlite".to_string()],
        "only the database probe runs when no provider was chosen"
    );
    assert!(out.contains("database reachable"), "probe label: {out}");
    assert!(out.contains("configuration valid"), "commit label: {out}");
    assert!(
        out.contains("saya --profile team"),
        "the next step names the new profile: {out}"
    );
    assert!(!dir.join(MARKER_FILE).exists(), "no marker is left");
    assert!(!dir.join(BACKUP_DIR).exists(), "no backups are left");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn cancel_at_confirm_writes_nothing() {
    let dir = temp_dir("cancel-confirm");
    let script = "6\n1\n/tmp/x.db\nteam\nn\n";
    let (code, out) = run_flow(
        &dir,
        script,
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "cancelling is not an error: {out}");
    assert!(
        out.contains("No files were changed."),
        "the cancel line: {out}"
    );
    assert!(
        !dir.join("connections.toml").exists(),
        "nothing was written"
    );
    assert!(!dir.join(MARKER_FILE).exists(), "no marker is left");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn eof_mid_flow_cancels_with_no_files() {
    let dir = temp_dir("eof-mid");
    // Provider skipped, then EOF at the database menu.
    let (code, out) = run_flow(
        &dir,
        "6\n",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "EOF cancels, never an error: {out}");
    assert!(out.contains("No files were changed."), "{out}");
    assert!(!dir.join("connections.toml").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn eof_at_the_first_prompt_cancels_with_no_files() {
    let dir = temp_dir("eof-first");
    let (code, out) = run_flow(
        &dir,
        "",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("No files were changed."), "{out}");
    assert!(!dir.join("connections.toml").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn existing_connections_shows_only_the_appended_block() {
    let dir = temp_dir("append");
    let original = "[profiles.a]\ntype = \"sqlite\"\npath = \"/tmp/a.db\"\nread_only = true\n";
    fs::write(dir.join("connections.toml"), original).unwrap();
    let script = "6\n1\n/tmp/b.db\nb\ny\n";
    let (code, out) = run_flow(
        &dir,
        script,
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("[profiles.b]"),
        "the appended block is shown: {out}"
    );
    assert!(
        !out.contains("[profiles.a]"),
        "the review shows only the appended block, never the whole file: {out}"
    );
    let parsed =
        ConnectionsFile::from_toml(&fs::read_to_string(dir.join("connections.toml")).unwrap())
            .unwrap();
    assert!(
        parsed.profiles.contains_key("a") && parsed.profiles.contains_key("b"),
        "the commit appends without dropping the existing profile"
    );
    let _ = fs::remove_dir_all(dir);
}
