#[test]
fn files_root_defaults_beside_the_state_db() {
    let root = isolated_root("default-files-root");
    let content = "id\n1\n";
    let csv = root.join("sales.csv");
    fs::write(&csv, content).unwrap();
    let sha = sha256_hex(content.as_bytes());
    let out = Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(["open", csv.to_str().unwrap(), "--non-interactive"])
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", &root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(
        snapshot_dirs(&root, &sha)
            .first()
            .is_some_and(|dir| dir.join("source.duckdb").exists()),
        "without SAYA_FILES_DIR the root is <state parent>/files"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn open_without_a_target_is_a_usage_error() {
    let root = isolated_root("usage");
    let bare = open_command(&root)
        .args(["open", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(
        bare.status.code(),
        Some(2),
        "nothing to do must be a usage error, stderr: {}",
        stderr_of(&bare)
    );
    let both = open_command(&root)
        .args(["open", "--list", "--cleanup", "all", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(
        both.status.code(),
        Some(2),
        "--list with --cleanup is a usage error, stderr: {}",
        stderr_of(&both)
    );
    let _ = fs::remove_dir_all(&root);
}
