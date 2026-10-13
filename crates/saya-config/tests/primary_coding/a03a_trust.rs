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
        "[coding]\nenabled = false\nmax_children = 3\nmax_concurrent_commands = 2\nmax_tool_calls = 299\nmax_job_tokens = 7\n",
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
    assert_eq!(
        resolved.coding.max_job_tokens, None,
        "the untrusted project ceiling is reverted to the unset default"
    );
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
        "coding.max_job_tokens",
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

#[test]
fn project_check_profiles_rollback_or_replace_as_one_trusted_map() {
    let user = ConfigFile::from_toml(
        "[coding.check_profiles.user]\ncommand_id='user-test'\nprogram='cargo'\nargv=['test']\ntimeout_seconds=60\nenvironment_profile='confined'\n",
    )
    .unwrap();
    let project = ConfigFile::from_toml(
        "[coding.check_profiles.project]\ncommand_id='project-test'\nprogram='cargo'\nargv=['check']\ntimeout_seconds=30\nenvironment_profile='confined'\n",
    )
    .unwrap();

    let ignored = resolve(
        empty()
            .with_user(user.clone())
            .with_project(project.clone()),
    )
    .unwrap();
    let ignored_diagnostics = serde_json::to_value(ignored.redacted_diagnostics()).unwrap();
    let ignored_profiles = ignored_diagnostics["coding_check_profiles"].clone();
    assert!(ignored_profiles.get("user").is_some());
    assert!(ignored_profiles.get("project").is_none());
    assert_eq!(ignored.ignored_project_overrides, ["coding"]);
    assert_eq!(
        ignored
            .provenance
            .get("coding.check_profiles")
            .unwrap()
            .source,
        SettingSource::UserFile
    );

    let trusted = resolve(
        empty()
            .with_user(user.clone())
            .with_project(project)
            .with_cli(CliOverrides {
                trust_project_config: true,
                ..Default::default()
            }),
    )
    .unwrap();
    let trusted_diagnostics = serde_json::to_value(trusted.redacted_diagnostics()).unwrap();
    let trusted_profiles = trusted_diagnostics["coding_check_profiles"].clone();
    assert!(trusted_profiles.get("user").is_none());
    assert!(trusted_profiles.get("project").is_some());
    assert_eq!(
        trusted
            .provenance
            .get("coding.check_profiles")
            .unwrap()
            .source,
        SettingSource::ProjectFile
    );

    let inherited = resolve(
        empty()
            .with_user(user.clone())
            .with_project(ConfigFile::from_toml("[coding]\nenabled=true\n").unwrap())
            .with_cli(CliOverrides {
                trust_project_config: true,
                ..Default::default()
            }),
    )
    .unwrap();
    let inherited_diagnostics = serde_json::to_value(inherited.redacted_diagnostics()).unwrap();
    assert!(
        inherited_diagnostics["coding_check_profiles"]
            .get("user")
            .is_some()
    );

    let cleared = resolve(
        empty()
            .with_user(user)
            .with_project(ConfigFile::from_toml("[coding]\ncheck_profiles = {}\n").unwrap())
            .with_cli(CliOverrides {
                trust_project_config: true,
                ..Default::default()
            }),
    )
    .unwrap();
    let cleared_diagnostics = serde_json::to_value(cleared.redacted_diagnostics()).unwrap();
    assert!(
        cleared_diagnostics["coding_check_profiles"]
            .as_object()
            .unwrap()
            .is_empty()
    );
}
