use saya_config::{
    CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, SettingSource, resolve,
};

fn empty() -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
}

#[test]
fn untrusted_project_coding_table_is_restored_with_all_provenance() {
    let user = ConfigFile::from_toml(
        "[coding]\nenabled = true\nmax_children = 1\nmax_concurrent_commands = 1\n",
    )
    .unwrap();
    let project = ConfigFile::from_toml(
        "[coding]\nenabled = false\nmax_children = 3\nmax_concurrent_commands = 2\nmax_tool_calls = 299\n",
    )
    .unwrap();
    let resolved = resolve(
        empty()
            .with_user(user.clone())
            .with_project(project.clone()),
    )
    .unwrap();
    assert!(resolved.coding.enabled);
    assert_eq!(resolved.coding.max_children, 1);
    assert_eq!(resolved.coding.max_concurrent_commands, 1);
    assert_eq!(resolved.coding.max_tool_calls, 300);
    assert_eq!(resolved.ignored_project_overrides, ["coding"]);
    for key in [
        "coding.enabled",
        "coding.max_children",
        "coding.max_concurrent_model_requests",
        "coding.max_concurrent_commands",
        "coding.max_logical_model_requests",
        "coding.max_physical_transport_attempts",
        "coding.max_tool_calls",
        "coding.max_known_tokens",
        "coding.max_elapsed_seconds",
    ] {
        assert_ne!(
            resolved.provenance.get(key).unwrap().source,
            SettingSource::ProjectFile,
            "untrusted project origin leaked for {key}"
        );
    }
    assert_eq!(
        resolved
            .provenance
            .get("coding.max_children")
            .unwrap()
            .source,
        SettingSource::UserFile
    );

    let trusted = resolve(
        empty()
            .with_user(user)
            .with_project(project)
            .with_cli(CliOverrides {
                trust_project_config: true,
                ..Default::default()
            }),
    )
    .unwrap();
    assert!(!trusted.coding.enabled);
    assert_eq!(trusted.coding.max_children, 3);
    assert_eq!(
        trusted
            .provenance
            .get("coding.max_children")
            .unwrap()
            .source,
        SettingSource::ProjectFile
    );
    assert!(trusted.ignored_project_overrides.is_empty());
}

#[test]
fn untrusted_project_coding_without_user_settings_restores_defaults() {
    let resolved = resolve(
        empty().with_project(
            ConfigFile::from_toml(
                "[coding]\nenabled = true\nmax_children = 1\nmax_concurrent_commands = 1\n",
            )
            .unwrap(),
        ),
    )
    .unwrap();
    assert!(!resolved.coding.enabled);
    assert_eq!(resolved.coding.max_children, 3);
    assert_eq!(resolved.coding.max_concurrent_commands, 2);
    assert_eq!(resolved.ignored_project_overrides, ["coding"]);
    for key in [
        "coding.enabled",
        "coding.max_children",
        "coding.max_concurrent_commands",
    ] {
        assert_eq!(
            resolved.provenance.get(key).unwrap().source,
            SettingSource::Default,
            "untrusted project origin leaked for {key}"
        );
    }
}
