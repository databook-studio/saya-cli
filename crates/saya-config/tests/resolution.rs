use saya_config::{CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, resolve};

#[test]
fn cli_values_override_every_other_source() {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml("[ai]\nmodel = 'user'\n").unwrap())
        .with_project(ConfigFile::from_toml("[ai]\nmodel = 'project'\n").unwrap())
        .with_env_file([("SAYA_AI_MODEL", "env-file")])
        .with_process_env([("SAYA_AI_MODEL", "process")])
        .with_cli(CliOverrides {
            model: Some("cli".into()),
            ..Default::default()
        });

    assert_eq!(resolve(input).unwrap().ai.model, "cli");
}

#[test]
fn profile_selection_prefers_flag_then_environment_then_config() {
    let connections = ConnectionsFile::from_toml(
        "[profiles.first]\ntype = 'duckdb'\npath = ':memory:'\n\
         [profiles.second]\ntype = 'duckdb'\npath = ':memory:'\n",
    )
    .unwrap();
    let config = ConfigFile::from_toml("default_profile = 'first'").unwrap();
    let env = ResolutionInput::new(connections)
        .with_user(config)
        .with_process_env([("SAYA_PROFILE", "second")]);
    assert_eq!(
        resolve(env).unwrap().profile_name.as_deref(),
        Some("second")
    );
}

#[test]
fn process_environment_overrides_the_explicit_env_file() {
    let input = ResolutionInput::new(ConnectionsFile::default())
        .with_env_file([("SAYA_AI_MODEL", "env-file")])
        .with_process_env([("SAYA_AI_MODEL", "process")]);
    assert_eq!(resolve(input).unwrap().ai.model, "process");
}

#[test]
fn multiple_profiles_require_explicit_selection() {
    let connections = ConnectionsFile::from_toml(
        "[profiles.first]\ntype = 'duckdb'\npath = ':memory:'\n\
         [profiles.second]\ntype = 'duckdb'\npath = ':memory:'\n",
    )
    .unwrap();
    assert!(resolve(ResolutionInput::new(connections)).is_err());
}

#[test]
fn a_single_profile_is_selected_without_an_explicit_default() {
    let connections =
        ConnectionsFile::from_toml("[profiles.local]\ntype = 'duckdb'\npath = ':memory:'\n")
            .unwrap();
    assert_eq!(
        resolve(ResolutionInput::new(connections))
            .unwrap()
            .profile_name
            .as_deref(),
        Some("local")
    );
}
