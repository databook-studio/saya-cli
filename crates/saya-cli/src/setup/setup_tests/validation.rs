#[cfg(unix)]
#[test]
fn symlink_refusal() {
    let dir = temp_dir("symlink");
    let outside = temp_dir("symlink-outside");
    fs::write(outside.join("elsewhere.toml"), "x = 1\n").unwrap();
    symlink(outside.join("elsewhere.toml"), dir.join("config.toml")).unwrap();
    fs::write(dir.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();

    let draft = SetupDraft {
        provider: Some(provider_draft()),
        profile: Some(profile_draft("c")),
    };
    let error = plan(&dir, &draft).unwrap_err();
    assert!(matches!(error, SetupError::Symlink { .. }));

    // A symlinked marker is refused too, and nothing is written.
    let dir2 = temp_dir("symlink-marker");
    fs::write(dir2.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();
    symlink(outside.join("elsewhere.toml"), dir2.join(MARKER_FILE)).unwrap();
    let planned = plan(&dir2, &draft).unwrap();
    let error = commit(&dir2, &planned, || Ok(())).unwrap_err();
    assert!(matches!(error, SetupError::Symlink { .. }));
    assert_eq!(
        fs::read(dir2.join("connections.toml")).unwrap(),
        EXISTING_CONNECTIONS.as_bytes(),
        "nothing was written"
    );

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(dir2);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn draft_validation_rejects_bad_names() {
    use super::draft::{validate_env_name, validate_profile_name};

    assert!(validate_env_name("OPENAI_API_KEY").is_ok());
    assert!(validate_env_name("_OK").is_ok());
    assert!(validate_env_name("lowercase").is_err());
    assert!(validate_env_name("1STARTS_DIGIT").is_err());
    assert!(validate_env_name("").is_err());
    assert!(validate_env_name(&"A".repeat(65)).is_err());
    assert!(validate_env_name(&"A".repeat(64)).is_ok());

    assert!(validate_profile_name("team.db-1").is_ok());
    assert!(validate_profile_name("bad name").is_err());
    assert!(validate_profile_name("").is_err());
    assert!(validate_profile_name(&"a".repeat(65)).is_err());
    assert!(validate_profile_name(&"a".repeat(64)).is_ok());

    let dir = fresh_dir("bad-draft");
    let draft = SetupDraft {
        provider: Some(ProviderDraft {
            provider: AiProvider::Openai,
            model: "m".into(),
            base_url: None,
            api_key_env: Some("oops".into()),
        }),
        profile: None,
    };
    assert!(matches!(plan(&dir, &draft), Err(SetupError::Draft(_))));
    assert!(!dir.exists(), "plan validates before touching anything");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn empty_draft_commit_is_noop() {
    let dir = fresh_dir("noop");
    let planned = plan(&dir, &SetupDraft::default()).unwrap();
    assert!(planned.writes.is_empty());
    assert!(planned.notes.is_empty());

    let reload_called = std::cell::Cell::new(false);
    let report = commit(&dir, &planned, || {
        reload_called.set(true);
        Ok(())
    })
    .unwrap();
    assert!(report.written.is_empty());
    assert!(
        !reload_called.get(),
        "a zero-write plan is a no-op: no reload"
    );
    assert!(!dir.exists(), "a zero-write plan creates nothing");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn corrupt_or_unknown_marker_is_reported() {
    let dir = temp_dir("marker-corrupt");
    fs::write(dir.join(MARKER_FILE), b"not json").unwrap();
    assert!(matches!(pending(&dir), Err(SetupError::Marker(_))));

    let dir = temp_dir("marker-version");
    let marker = serde_json::json!({ "version": 2, "started_unix_ms": 1, "entries": [] });
    fs::write(dir.join(MARKER_FILE), serde_json::to_vec(&marker).unwrap()).unwrap();
    assert!(matches!(pending(&dir), Err(SetupError::Marker(_))));

    let dir = fresh_dir("marker-absent");
    assert!(pending(&dir).unwrap().is_none());
    let _ = fs::remove_dir_all(dir);
    for label in ["marker-corrupt", "marker-version"] {
        let _ = fs::remove_dir_all(temp_dir(label));
    }
}

#[test]
fn commit_refuses_when_commit_pending() {
    let dir = temp_dir("commit-pending");
    fs::write(dir.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();
    stage_interrupted_commit(
        &dir,
        EXISTING_CONNECTIONS,
        "[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n",
    );
    let draft = SetupDraft {
        provider: None,
        profile: Some(profile_draft("d")),
    };
    let planned = plan(&dir, &draft).unwrap();
    let error = commit(&dir, &planned, || Ok(())).unwrap_err();
    assert!(matches!(error, SetupError::CommitPending { .. }));
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        b"[profiles.c]\ntype = \"sqlite\"\npath = \"/tmp/c.db\"\nread_only = true\n",
        "the refused commit touched nothing"
    );
    let _ = fs::remove_dir_all(dir);
}
