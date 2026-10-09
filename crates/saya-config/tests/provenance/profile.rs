//! `profile_name` provenance mirrors the selection chain: the CLI flag, the
//! process environment, a declared default, and single-profile inference.

use saya_config::{
    CliOverrides, ConfigFile, ConnectionsFile, ResolutionInput, SettingSource, resolve,
};

use super::source_of;

#[test]
fn profile_name_provenance_mirrors_the_selection_chain() {
    let connections =
        ConnectionsFile::from_toml("[profiles.local]\ntype = 'duckdb'\npath = ':memory:'\n")
            .unwrap();

    let resolved = resolve(
        ResolutionInput::new(connections.clone()).with_cli(CliOverrides {
            profile: Some("local".into()),
            ..Default::default()
        }),
    )
    .unwrap();
    assert_eq!(resolved.profile_name.as_deref(), Some("local"));
    assert_eq!(
        source_of(&resolved, "profile_name"),
        SettingSource::Cli {
            flag: "--profile".into()
        }
    );

    let resolved = resolve(
        ResolutionInput::new(connections.clone()).with_process_env([("SAYA_PROFILE", "local")]),
    )
    .unwrap();
    assert_eq!(
        source_of(&resolved, "profile_name"),
        SettingSource::Env {
            var: "SAYA_PROFILE".into()
        }
    );

    let resolved = resolve(
        ResolutionInput::new(connections.clone())
            .with_user(ConfigFile::from_toml("default_profile = 'local'\n").unwrap()),
    )
    .unwrap();
    assert_eq!(
        source_of(&resolved, "profile_name"),
        SettingSource::UserFile
    );

    // Single-profile inference records Default.
    let resolved = resolve(ResolutionInput::new(connections)).unwrap();
    assert_eq!(resolved.profile_name.as_deref(), Some("local"));
    assert_eq!(source_of(&resolved, "profile_name"), SettingSource::Default);
}
