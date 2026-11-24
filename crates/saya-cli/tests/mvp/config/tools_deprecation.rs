/// A `[tools] allow` list that still spells an old name launches — the list is
/// honoured, never an error — and `config doctor` names the stale word and its
/// successor, so a user learns to fix it from the command that reports on their
/// setup rather than from a refusal.
#[test]
fn config_doctor_reports_a_deprecated_tools_allow_spelling() {
    let root = test_root("doctor-tools-deprecated");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[tools]\nallow = [\"run_command\", \"read\"]\n",
    )
    .unwrap();
    let output = saya_isolated(&root, &["config", "doctor"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("unknown tool"),
        "an old spelling is not a refusal: {stdout}"
    );
    assert!(
        stdout.contains("[tools] allow:")
            && stdout.contains("run_command")
            && stdout.contains("shell"),
        "doctor names the deprecated spelling and its successor: {stdout}"
    );
    let _ = fs::remove_dir_all(root);
}

/// A list of current names adds no such line: doctor reports what is stale and
/// nothing else.
#[test]
fn config_doctor_is_silent_about_a_current_tools_allow_list() {
    let root = test_root("doctor-tools-current");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[tools]\nallow = [\"shell\", \"read\"]\n",
    )
    .unwrap();
    let output = saya_isolated(&root, &["config", "doctor"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("deprecated spelling"),
        "a current list states no deprecation: {stdout}"
    );
    let _ = fs::remove_dir_all(root);
}
