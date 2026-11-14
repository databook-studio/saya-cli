use super::*;

#[test]
fn environment_only_profiles_support_sqlite() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_process_env([("SAYA_DB_TYPE", "sqlite"), ("SAYA_DB_PATH", "data.db")]),
    )
    .unwrap();
    assert!(matches!(
        resolved.profile,
        Some(DatabaseProfile::Sqlite {
            ref path,
            read_only: true,
        }) if path == "data.db"
    ));
}

#[test]
fn sqlite_read_only_environment_override_is_typed() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_DB_TYPE", "sqlite"),
            ("SAYA_DB_PATH", "data.db"),
            ("SAYA_DB_READ_ONLY", "true"),
        ]),
    )
    .unwrap();
    assert!(matches!(
        resolved.profile,
        Some(DatabaseProfile::Sqlite {
            read_only: true,
            ..
        })
    ));
}

#[test]
fn sqlite_read_only_environment_rejects_non_boolean_values() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_DB_TYPE", "sqlite"),
            ("SAYA_DB_PATH", "data.db"),
            ("SAYA_DB_READ_ONLY", "notabool"),
        ]),
    )
    .unwrap_err();
    assert!(
        matches!(error, ConfigError::InvalidEnvironment { name, .. } if name == "SAYA_DB_READ_ONLY")
    );
}

#[test]
fn sqlite_path_environment_overlay_preserves_profile_read_only_setting() {
    let profiles = ConnectionsFile::from_toml(
        "[profiles.local]\ntype = 'sqlite'\npath = 'file.db'\nread_only = true\n",
    )
    .unwrap();
    let resolved = resolve(
        ResolutionInput::new(profiles)
            .with_process_env([("SAYA_PROFILE", "local"), ("SAYA_DB_PATH", "override.db")]),
    )
    .unwrap();

    assert!(matches!(
        resolved.profile,
        Some(DatabaseProfile::Sqlite { ref path, read_only: true }) if path == "override.db"
    ));
}
