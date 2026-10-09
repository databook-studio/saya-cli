use super::*;

#[test]
fn duckdb_path_environment_overlay_preserves_profile_read_only_setting() {
    let profiles = ConnectionsFile::from_toml(
        "[profiles.local]\ntype = 'duckdb'\npath = 'base.duckdb'\nread_only = true\n",
    )
    .unwrap();
    let resolved = resolve(ResolutionInput::new(profiles).with_process_env([
        ("SAYA_PROFILE", "local"),
        ("SAYA_DB_PATH", "override.duckdb"),
    ]))
    .unwrap();

    assert!(matches!(
        resolved.profile,
        Some(DatabaseProfile::DuckDb { ref path, read_only: Some(true) }) if path == "override.duckdb"
    ));
}

#[test]
fn duckdb_read_only_environment_override_is_typed() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_DB_TYPE", "duckdb"),
            ("SAYA_DB_PATH", "data.duckdb"),
            ("SAYA_DB_READ_ONLY", "true"),
        ]),
    )
    .unwrap();
    assert!(matches!(
        resolved.profile,
        Some(DatabaseProfile::DuckDb {
            read_only: Some(true),
            ..
        })
    ));
}

#[test]
fn duckdb_read_only_environment_rejects_non_boolean_values() {
    let error = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_DB_TYPE", "duckdb"),
            ("SAYA_DB_PATH", "data.duckdb"),
            ("SAYA_DB_READ_ONLY", "yes"),
        ]),
    )
    .unwrap_err();
    assert!(
        matches!(error, ConfigError::InvalidEnvironment { name, .. } if name == "SAYA_DB_READ_ONLY")
    );
}
