use saya_config::{ConfigError, ConnectionsFile, ResolutionInput, resolve};
use saya_types::{DatabaseProfile, SecretRef};

#[test]
fn builds_a_postgres_profile_from_environment_without_retaining_the_password() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_DB_TYPE", "postgresql"),
            ("SAYA_DB_HOST", "db.internal"),
            ("SAYA_DB_PORT", "5433"),
            ("SAYA_DB_NAME", "warehouse"),
            ("SAYA_DB_USER", "readonly"),
            ("SAYA_DB_PASSWORD", "value-never-stored"),
        ]),
    )
    .unwrap();

    assert!(matches!(
        resolved.profile,
        Some(DatabaseProfile::Postgres { password: Some(SecretRef::Env { ref env }), .. })
            if env == "SAYA_DB_PASSWORD"
    ));
    assert!(!format!("{resolved:?}").contains("value-never-stored"));
}

#[test]
fn named_environment_profile_succeeds_without_a_connections_file_entry() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_PROFILE", "analytics"),
            ("SAYA_DB_TYPE", "postgresql"),
            ("SAYA_DB_HOST", "db.internal"),
            ("SAYA_DB_NAME", "warehouse"),
            ("SAYA_DB_USER", "readonly"),
        ]),
    )
    .unwrap();

    assert_eq!(resolved.profile_name.as_deref(), Some("analytics"));
    assert!(matches!(
        resolved.profile,
        Some(DatabaseProfile::Postgres { .. })
    ));
}

#[test]
fn environment_database_fields_overlay_env_file_then_named_profile() {
    let profiles = ConnectionsFile::from_toml(
        "[profiles.analytics]\ntype = 'postgresql'\nhost = 'profile-host'\n\
         port = 5432\ndatabase = 'warehouse'\nuser = 'readonly'\n",
    )
    .unwrap();
    let resolved = resolve(
        ResolutionInput::new(profiles)
            .with_env_file([("SAYA_DB_HOST", "env-file-host"), ("SAYA_DB_PORT", "5434")])
            .with_process_env([
                ("SAYA_PROFILE", "analytics"),
                ("SAYA_DB_HOST", "process-host"),
                ("SAYA_DB_PORT", "5435"),
            ]),
    )
    .unwrap();

    assert!(matches!(
        resolved.profile,
        Some(DatabaseProfile::Postgres { ref host, port: Some(5435), .. })
            if host == "process-host"
    ));
}

#[test]
fn environment_profiles_report_typed_configuration_errors() {
    let missing = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_process_env([("SAYA_DB_TYPE", "postgresql")]),
    )
    .unwrap_err();
    assert!(matches!(missing, ConfigError::MissingDatabaseField { .. }));

    let unsupported = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_process_env([("SAYA_DB_TYPE", "oracle")]),
    )
    .unwrap_err();
    assert!(matches!(
        unsupported,
        ConfigError::UnsupportedDatabaseType(_)
    ));
}

#[test]
fn environment_only_profiles_support_mysql_and_duckdb() {
    let mysql = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_DB_TYPE", "mysql"),
            ("SAYA_DB_HOST", "mysql.internal"),
            ("SAYA_DB_NAME", "warehouse"),
            ("SAYA_DB_USER", "readonly"),
        ]),
    )
    .unwrap();
    assert!(matches!(mysql.profile, Some(DatabaseProfile::Mysql { .. })));

    let duckdb = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_process_env([("SAYA_DB_TYPE", "duckdb"), ("SAYA_DB_PATH", "data.duckdb")]),
    )
    .unwrap();
    assert!(matches!(
        duckdb.profile,
        Some(DatabaseProfile::DuckDb { .. })
    ));
}

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
