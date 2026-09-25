use saya_config::{CompactionMode, ConfigFile, ConnectionsFile, ResolutionInput, resolve};

#[test]
fn compaction_defaults_to_auto_and_resolves_each_spelling() {
    let defaults = resolve(ResolutionInput::new(ConnectionsFile::default())).unwrap();
    assert_eq!(defaults.ai.compaction, CompactionMode::Auto);

    for (toml, expected) in [
        ("[ai]\ncompaction = 'auto'\n", CompactionMode::Auto),
        ("[ai]\ncompaction = 'manual'\n", CompactionMode::Manual),
        ("[ai]\ncompaction = 'off'\n", CompactionMode::Off),
    ] {
        let config = ConfigFile::from_toml(toml).unwrap();
        let resolved =
            resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config)).unwrap();
        assert_eq!(resolved.ai.compaction, expected, "for {toml:?}");
    }
}

#[test]
fn compaction_follows_the_existing_config_layering() {
    // Project over user, like every other `[ai]` scalar.
    let user = ConfigFile::from_toml("[ai]\ncompaction = 'manual'\n").unwrap();
    let project = ConfigFile::from_toml("[ai]\ncompaction = 'off'\n").unwrap();
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(user)
            .with_project(project),
    )
    .unwrap();
    assert_eq!(resolved.ai.compaction, CompactionMode::Off);

    // The environment wins over files.
    let file = ConfigFile::from_toml("[ai]\ncompaction = 'off'\n").unwrap();
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(file)
            .with_process_env([("SAYA_COMPACTION", "manual")]),
    )
    .unwrap();
    assert_eq!(resolved.ai.compaction, CompactionMode::Manual);
}

#[test]
fn compaction_spelling_round_trips_and_rejects_unknown_words() {
    for mode in [
        CompactionMode::Auto,
        CompactionMode::Manual,
        CompactionMode::Off,
    ] {
        assert_eq!(CompactionMode::parse(mode.as_str()), Some(mode));
    }
    assert_eq!(CompactionMode::parse("AUTO"), None);
    assert_eq!(CompactionMode::parse("sometimes"), None);
    assert!(ConfigFile::from_toml("[ai]\ncompaction = 'sometimes'\n").is_err());
}

#[test]
fn compaction_is_visible_in_config_show_and_doctor() {
    let config = ConfigFile::from_toml("[ai]\ncompaction = 'manual'\n").unwrap();
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config)).unwrap();
    let rendered = serde_json::to_string(&resolved.redacted_diagnostics()).unwrap();
    assert!(
        rendered.contains("\"compaction\":\"manual\""),
        "config show must carry the setting: {rendered}"
    );
    let declared = ConfigFile::from_toml("[ai]\ncompaction = 'off'\n").unwrap();
    assert_eq!(
        declared.redacted_diagnostics().compaction,
        Some(CompactionMode::Off)
    );
    let unset = ConfigFile::from_toml("[ai]\nmodel = 'x'\n").unwrap();
    assert_eq!(unset.redacted_diagnostics().compaction, None);
}
