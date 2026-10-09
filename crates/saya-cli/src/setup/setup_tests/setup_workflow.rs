#[test]
fn setup_cancel_preserves_files() {
    let dir = temp_dir("cancel");
    let config_before = b"default_profile = \"a\"\n\n[run]\nmax_rows = 10\n".to_vec();
    let connections_before = EXISTING_CONNECTIONS.as_bytes().to_vec();
    fs::write(dir.join("config.toml"), &config_before).unwrap();
    fs::write(dir.join("connections.toml"), &connections_before).unwrap();

    let planned = plan(
        &dir,
        &SetupDraft {
            provider: None,
            profile: Some(profile_draft("c")),
        },
    )
    .unwrap();
    assert_eq!(planned.writes.len(), 1);

    assert_eq!(fs::read(dir.join("config.toml")).unwrap(), config_before);
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        connections_before
    );
    assert!(!dir.join(MARKER_FILE).exists());

    let fresh = fresh_dir("cancel-fresh");
    plan(&fresh, &SetupDraft::default()).unwrap();
    assert!(!fresh.exists(), "plan must not create the directory");

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(fresh);
}

#[test]
fn setup_preserves_existing_profile() {
    let dir = temp_dir("preserve");
    fs::write(dir.join("connections.toml"), EXISTING_CONNECTIONS).unwrap();

    let exists = plan(
        &dir,
        &SetupDraft {
            provider: None,
            profile: Some(profile_draft("a")),
        },
    );
    assert!(matches!(exists, Err(SetupError::ProfileExists(name)) if name == "a"));
    assert_eq!(
        fs::read(dir.join("connections.toml")).unwrap(),
        EXISTING_CONNECTIONS.as_bytes(),
        "the refused plan still touched nothing"
    );

    let planned = plan(
        &dir,
        &SetupDraft {
            provider: None,
            profile: Some(profile_draft("c")),
        },
    )
    .unwrap();
    assert_eq!(planned.writes.len(), 1);
    let write = &planned.writes[0];
    assert_eq!(write.file, "connections.toml");
    assert!(!write.created);
    let content = &write.content;
    assert!(
        content.starts_with(EXISTING_CONNECTIONS),
        "existing bytes are an exact prefix"
    );
    let suffix = &content[EXISTING_CONNECTIONS.len()..];
    assert!(
        suffix.starts_with("\n[profiles.c]"),
        "the original ends with a newline, so one blank line then the block"
    );
    let parsed = ConnectionsFile::from_toml(content).unwrap();
    assert!(parsed.profiles.contains_key("a"));
    assert!(parsed.profiles.contains_key("b"));
    assert!(parsed.profiles.contains_key("c"));

    // A file without a trailing newline gets one before the appended block.
    let dir2 = temp_dir("preserve-no-newline");
    let raw = "# only b\n[profiles.b]\ntype = \"duckdb\"\npath = \"/tmp/b.duckdb\"";
    fs::write(dir2.join("connections.toml"), raw).unwrap();
    let planned = plan(
        &dir2,
        &SetupDraft {
            provider: None,
            profile: Some(profile_draft("c")),
        },
    )
    .unwrap();
    let content = &planned.writes[0].content;
    assert!(content.starts_with(raw));
    assert!(content[raw.len()..].starts_with("\n\n[profiles.c]"));
    assert!(ConnectionsFile::from_toml(content).is_ok());

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(dir2);
}

#[test]
fn setup_existing_config_is_never_modified() {
    let dir = temp_dir("config-untouched");
    let before =
        b"default_profile = \"analytics\"\n\n[run]\nread_only = true\nmax_rows = 1000\n".to_vec();
    fs::write(dir.join("config.toml"), &before).unwrap();

    let draft = SetupDraft {
        provider: Some(provider_draft()),
        profile: Some(profile_draft("team")),
    };
    let planned = plan(&dir, &draft).unwrap();
    assert_eq!(planned.writes.len(), 1, "only connections.toml is written");
    assert_eq!(planned.writes[0].file, "connections.toml");
    assert_eq!(planned.notes.len(), 1);
    let note = &planned.notes[0];
    assert!(note.contains("[ai]"));
    assert!(note.contains("provider = \"openai\""));
    assert!(note.contains("api_key = { env = \"OPENAI_API_KEY\" }"));

    let report = commit(&dir, &planned, || Ok(())).unwrap();
    assert_eq!(report.written, vec!["connections.toml".to_string()]);
    assert_eq!(
        fs::read(dir.join("config.toml")).unwrap(),
        before,
        "config.toml bytes unchanged after commit"
    );
    assert!(dir.join("connections.toml").exists());
    let _ = fs::remove_dir_all(dir);
}
