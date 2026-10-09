#[test]
fn three_invalid_answers_cancel() {
    let dir = temp_dir("three-bad");
    let (code, out) = run_flow(
        &dir,
        "bogus\nbogus\nbogus\n",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "three strikes cancel, not error: {out}");
    assert!(out.contains("No files were changed."), "{out}");
    assert!(!dir.join("connections.toml").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn failed_database_probe_does_not_block_writing() {
    let dir = temp_dir("probe-fail");
    let (code, out) = run_flow(
        &dir,
        SQLITE_SCRIPT,
        instant_probes(&Arc::new(Mutex::new(Vec::new())), false, true),
    );
    assert_eq!(code, 0, "a failed probe does not block writing: {out}");
    assert!(
        out.contains("database probe failed: connection refused"),
        "the failure is shown verbatim: {out}"
    );
    assert!(out.contains("configuration valid"), "{out}");
    assert!(
        dir.join("connections.toml").exists(),
        "the file was written"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn provider_probe_needs_consent_and_skips_without_it() {
    let dir = temp_dir("consent-no");
    // Ollama (1), model blank (default), base URL blank (default); sqlite,
    // path, name; decline the provider probe (n); confirm the write (y).
    let script = "1\n\n\n1\n/tmp/x.db\nteam\nn\ny\n";
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (code, out) = run_flow(&dir, script, instant_probes(&calls, true, true));
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        calls.lock().unwrap().clone(),
        vec!["database:sqlite".to_string()],
        "the provider probe never runs without consent"
    );
    assert!(out.contains("Provider probe skipped"), "{out}");
    assert!(dir.join("connections.toml").exists());
    assert!(
        dir.join("config.toml").exists(),
        "the ollama section is written"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn provider_probe_runs_with_consent() {
    let dir = temp_dir("consent-yes");
    let script = "1\n\n\n1\n/tmp/x.db\nteam\ny\ny\n";
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (code, out) = run_flow(&dir, script, instant_probes(&calls, true, true));
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        calls.lock().unwrap().clone(),
        vec!["database:sqlite".to_string(), "provider:ollama".to_string()],
        "the consented probe runs after the database probe"
    );
    assert!(out.contains("provider answered"), "{out}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn skipping_everything_changes_nothing() {
    let dir = temp_dir("skip-all");
    let (code, out) = run_flow(
        &dir,
        "6\n8\n",
        instant_probes(&Arc::new(Mutex::new(Vec::new())), true, true),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Nothing to configure"), "{out}");
    assert!(!dir.join("connections.toml").exists());
    assert!(!dir.join("config.toml").exists());
}
