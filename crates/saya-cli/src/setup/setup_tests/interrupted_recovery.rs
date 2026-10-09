#[test]
fn setup_recovers_interrupted_pair_write() {
    let original = EXISTING_CONNECTIONS;
    let rewritten = format!(
        "{original}\n[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n"
    );

    let dir = temp_dir("recover-restore");
    fs::write(dir.join("connections.toml"), original).unwrap();
    stage_interrupted_commit(&dir, original, &rewritten);

    let found = pending(&dir).unwrap().expect("the marker is found");
    assert_eq!(found.started_unix_ms, 42);
    assert_eq!(found.entries.len(), 2);

    restore(&dir, &found).unwrap();
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        original.as_bytes()
    );
    assert!(
        !dir.join("config.toml").exists(),
        "the created file is removed"
    );
    assert!(!dir.join(MARKER_FILE).exists());
    assert!(!dir.join(BACKUP_DIR).exists());
    assert!(pending(&dir).unwrap().is_none(), "no marker left");
    restore(&dir, &found).unwrap(); // idempotent: a second call is a no-op
    let _ = fs::remove_dir_all(dir);

    let dir = temp_dir("recover-finish");
    fs::write(dir.join("connections.toml"), original).unwrap();
    stage_interrupted_commit(&dir, original, &rewritten);
    let found = pending(&dir).unwrap().unwrap();
    finish(&dir, &found).unwrap();
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        rewritten.as_bytes(),
        "finish keeps the new bytes"
    );
    assert!(!dir.join(MARKER_FILE).exists());
    assert!(!dir.join(BACKUP_DIR).exists());
    finish(&dir, &found).unwrap(); // idempotent
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn reload_failure_restores_originals() {
    let dir = temp_dir("reload-fail");
    fs::write(dir.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();

    let draft = SetupDraft {
        provider: Some(provider_draft()),
        profile: Some(profile_draft("c")),
    };
    let planned = plan(&dir, &draft).unwrap();
    let error = commit(&dir, &planned, || Err("probe failed".into())).unwrap_err();
    assert!(matches!(error, SetupError::ReloadFailed(message) if message.contains("probe failed")));

    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        EXISTING_CONNECTIONS.as_bytes()
    );
    assert!(
        !dir.join("config.toml").exists(),
        "the created file is deleted"
    );
    assert!(!dir.join(MARKER_FILE).exists());
    assert!(!dir.join(BACKUP_DIR).exists());
    let _ = fs::remove_dir_all(dir);
}

/// F6: a backup copy that fails before any target is published must not leave
/// the marker behind — the next startup would otherwise warn about an
/// "interrupted setup" that never modified anything. The failure is injected
/// by taking the backup directory's name with a regular file, so the first
/// backup copy cannot be written.
#[test]
fn backup_failure_before_any_publish_removes_the_marker() {
    let dir = temp_dir("backup-fail");
    fs::write(dir.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();
    fs::write(dir.join(BACKUP_DIR), b"not a directory\n").unwrap();

    let draft = SetupDraft {
        provider: Some(provider_draft()),
        profile: Some(profile_draft("c")),
    };
    let planned = plan(&dir, &draft).unwrap();
    let error = commit(&dir, &planned, || {
        panic!("reload must not run when the backups failed")
    })
    .unwrap_err();
    assert!(
        matches!(error, SetupError::Io { .. }),
        "the injected backup failure surfaces: {error:?}"
    );
    assert!(
        !dir.join(MARKER_FILE).exists(),
        "no marker is left: no target was ever published"
    );
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        EXISTING_CONNECTIONS.as_bytes(),
        "the existing target is unchanged"
    );
    assert!(
        !dir.join("config.toml").exists(),
        "no target was created either"
    );
    let _ = fs::remove_dir_all(dir);
}

/// F7: when the reload fails and the automatic restore ALSO fails, the error
/// must not claim the originals were restored. It says the restore did not
/// complete and that `saya setup` will offer a restore on the next run — which
/// only works if the marker stays.
#[cfg(unix)]
#[test]
fn reload_failure_with_failed_restore_keeps_the_marker() {
    let dir = temp_dir("reload-restore-fail");
    fs::write(dir.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();
    let outside = temp_dir("reload-restore-fail-outside");
    fs::write(outside.join("sentinel.toml"), "sentinel = 1\n").unwrap();

    let draft = SetupDraft {
        provider: Some(provider_draft()),
        profile: Some(profile_draft("c")),
    };
    let planned = plan(&dir, &draft).unwrap();
    let backup = dir.join(BACKUP_DIR).join("connections.toml");
    let error = commit(&dir, &planned, || {
        // Sabotage the restore that would follow the reload failure: the
        // backup becomes a symlink, which restore refuses — and never follows.
        fs::remove_file(&backup).unwrap();
        symlink(outside.join("sentinel.toml"), &backup).unwrap();
        Err("probe failed".into())
    })
    .unwrap_err();
    let text = error.to_string();
    assert!(
        matches!(&error, SetupError::ReloadRestoreFailed { message, restore }
            if message.contains("probe failed") && !restore.is_empty()),
        "a distinct error reports the failed restore: {error:?}"
    );
    assert!(
        !text.contains("were restored"),
        "the error never claims the originals were restored: {text}"
    );
    assert!(
        text.contains("did NOT complete"),
        "the message says the restore did not complete: {text}"
    );
    assert!(
        text.contains("saya setup"),
        "the message names the next-run restore offer: {text}"
    );
    assert!(
        pending(&dir).unwrap().is_some(),
        "the marker remains so the next run really offers the restore"
    );
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        planned.writes[0].content.as_bytes(),
        "the sabotaged backup was never published: the target keeps the new bytes"
    );
    assert!(
        !dir.join("config.toml").exists(),
        "the created file was removed by the step that could succeed"
    );
    assert_eq!(
        fs::read_to_string(outside.join("sentinel.toml")).unwrap(),
        "sentinel = 1\n",
        "the sabotaged backup symlink was never followed"
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(outside);
}
