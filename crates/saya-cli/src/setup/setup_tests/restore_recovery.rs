/// A restore step that fails must not clear the marker or the backups: the
/// pending state stays until a later, fixed retry really completes, and the
/// failing entry is named. A step that could succeed still runs.
#[test]
fn restore_failure_keeps_marker_and_reports() {
    let dir = temp_dir("restore-fail");
    let original = EXISTING_CONNECTIONS;
    let rewritten = format!(
        "{original}\n[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n"
    );
    fs::write(dir.join("connections.toml"), original).unwrap();
    stage_interrupted_commit(&dir, original, &rewritten);
    // The created target was replaced by a directory after the interruption:
    // removing it cannot succeed.
    fs::create_dir_all(dir.join("config.toml")).unwrap();

    let found = pending(&dir).unwrap().expect("the marker is found");
    let error = restore(&dir, &found).unwrap_err();
    assert!(
        matches!(&error, SetupError::RestoreIncomplete { failures }
            if failures.iter().any(|failure| failure.starts_with("config.toml:"))),
        "the failing entry is named: {error:?}"
    );
    assert!(dir.join(MARKER_FILE).exists(), "the marker is kept");
    assert!(dir.join(BACKUP_DIR).exists(), "the backups are kept");
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        original.as_bytes(),
        "the step that could succeed still ran"
    );

    // Retry after fixing the obstacle: restore really completes and clears.
    fs::remove_dir_all(dir.join("config.toml")).unwrap();
    let report = restore(&dir, &found).unwrap();
    assert!(
        !dir.join(MARKER_FILE).exists(),
        "the retry clears the marker"
    );
    assert!(
        !dir.join(BACKUP_DIR).exists(),
        "the retry clears the backups"
    );
    assert!(
        !dir.join("config.toml").exists(),
        "the created file is removed"
    );
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        original.as_bytes()
    );
    assert!(
        report.outcomes.iter().any(
            |outcome| matches!(outcome, RestoreOutcome::Removed { file } if file == "config.toml")
        ),
        "the retry reports the removal: {:?}",
        report.outcomes
    );
    let _ = fs::remove_dir_all(dir);
}

/// A missing backup means the commit never reached publishing (it writes
/// every backup before any publish), so the target is left untouched and the
/// report says so — it must not be claimed as restored.
#[test]
fn missing_backup_before_publish_is_a_truthful_skip() {
    let dir = temp_dir("backup-absent");
    let original = EXISTING_CONNECTIONS;
    fs::write(dir.join("connections.toml"), original).unwrap();
    // Marker written, but the crash hit before any backup copy and before any
    // publish: the entry claims a backup that was never written, and the
    // created second file does not exist yet.
    let marker = serde_json::json!({
        "version": 1,
        "started_unix_ms": 42,
        "entries": [
            { "file": "connections.toml", "backup": "connections.toml", "created": false },
            { "file": "config.toml", "backup": null, "created": true }
        ]
    });
    fs::write(dir.join(MARKER_FILE), serde_json::to_vec(&marker).unwrap()).unwrap();

    let found = pending(&dir).unwrap().unwrap();
    let report = restore(&dir, &found).unwrap();
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        original.as_bytes(),
        "the target is untouched"
    );
    assert!(
        report.outcomes.contains(&RestoreOutcome::LeftUnchanged {
            file: "connections.toml".into(),
            reason: "backup absent; commit had not modified it".into(),
        }),
        "the skip is recorded truthfully: {:?}",
        report.outcomes
    );
    assert!(
        !dir.join("config.toml").exists(),
        "the created entry has nothing to remove"
    );
    assert!(
        !dir.join(MARKER_FILE).exists(),
        "a complete restore clears the marker"
    );
    assert!(
        !dir.join(BACKUP_DIR).exists(),
        "a complete restore clears the backups"
    );
    let _ = fs::remove_dir_all(dir);
}

/// A missing backup is only a truthful skip while no publishing had started.
/// When another entry's backup exists and differs from its target, publishing
/// had begun: the state is inconsistent, the entry fails, and the marker and
/// backups stay.
#[test]
fn inconsistent_missing_backup_is_a_failure() {
    let dir = temp_dir("backup-inconsistent");
    let rewritten = format!(
        "{EXISTING_CONNECTIONS}\n[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\n\
         read_only = true\n"
    );
    fs::write(dir.join("connections.toml"), &rewritten).unwrap();
    let published = "[ai]\nprovider = \"ollama\"\n";
    let pre_commit = "[ai]\nprovider = \"openai\"\n";
    fs::write(dir.join("config.toml"), published).unwrap();
    // connections.toml's backup is gone; config.toml's backup is present and
    // differs from the current target: publishing had started.
    let marker = serde_json::json!({
        "version": 1,
        "started_unix_ms": 42,
        "entries": [
            { "file": "connections.toml", "backup": "connections.toml", "created": false },
            { "file": "config.toml", "backup": "config.toml", "created": false }
        ]
    });
    fs::write(dir.join(MARKER_FILE), serde_json::to_vec(&marker).unwrap()).unwrap();
    fs::create_dir_all(dir.join(BACKUP_DIR)).unwrap();
    fs::write(dir.join(BACKUP_DIR).join("config.toml"), pre_commit).unwrap();

    let found = pending(&dir).unwrap().unwrap();
    let error = restore(&dir, &found).unwrap_err();
    assert!(
        matches!(&error, SetupError::RestoreIncomplete { failures }
            if failures.iter().any(|failure| failure.starts_with("connections.toml:"))),
        "the inconsistent entry fails: {error:?}"
    );
    assert!(dir.join(MARKER_FILE).exists(), "the marker is kept");
    assert!(dir.join(BACKUP_DIR).exists(), "the backups are kept");
    // The entry with a present backup still gets restored: steps continue.
    assert_eq!(
        fs::read(dir.join("config.toml")).unwrap(),
        pre_commit.as_bytes()
    );
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        rewritten.as_bytes(),
        "the inconsistent entry was left untouched"
    );
    let _ = fs::remove_dir_all(dir);
}

/// A symlink planted at the predictable staging name must never be followed:
/// the victim file keeps its bytes and the target still lands.
#[cfg(unix)]
#[test]
fn publish_does_not_follow_a_symlink_planted_at_the_staging_name() {
    let dir = temp_dir("publish-staging-symlink");
    let victim = dir.join("victim.txt");
    fs::write(&victim, "keep me").unwrap();
    let staging = dir.join(format!(".config.toml.{}.tmp", std::process::id()));
    symlink(&victim, &staging).unwrap();
    let target = dir.join("config.toml");
    super::atomic::publish(&target, b"fresh bytes").unwrap();
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep me");
    assert_eq!(fs::read_to_string(&target).unwrap(), "fresh bytes");
    let _ = fs::remove_dir_all(&dir);
}
