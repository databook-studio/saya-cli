use saya_config::{ConfigError, ConnectionsFile, ResolutionInput, resolve};
use saya_types::{DatabaseProfile, MySqlSslMode, SecretRef};

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
fn mysql_tls_environment_overrides_profile_without_retaining_secret_values() {
    let profile = ConnectionsFile::from_toml(
        "[profiles.mysql]\ntype = 'mysql'\nhost = 'db'\ndatabase = 'app'\nuser = 'reader'\nsslmode = 'require'\n",
    ).unwrap();
    let resolved = resolve(ResolutionInput::new(profile).with_process_env([
        ("SAYA_PROFILE", "mysql"),
        ("SAYA_DB_SSLMODE", "verify-identity"),
        ("SAYA_DB_SSL_CA", "ca-secret-sentinel"),
    ]))
    .unwrap();
    assert!(matches!(resolved.profile, Some(DatabaseProfile::Mysql {
        ssl_mode: Some(MySqlSslMode::VerifyIdentity),
        ssl_ca: Some(SecretRef::Env { ref env }), ..
    }) if env == "SAYA_DB_SSL_CA"));
    assert!(!format!("{resolved:?}").contains("ca-secret-sentinel"));
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

#[test]
fn snowflake_environment_overlay_preserves_profile_fields_and_only_stores_secret_refs() {
    let profiles = ConnectionsFile::from_toml(
        "[profiles.analytics]\ntype = 'snowflake'\naccount = 'org-account.us-east-1.aws'\nuser = 'profile-user'\nauth_type = 'userpass'\npassword = { env = 'PROFILE_PASSWORD' }\nwarehouse = 'profile_wh'\n",
    )
    .unwrap();
    let resolved = resolve(
        ResolutionInput::new(profiles)
            .with_env_file([
                ("SAYA_PROFILE", "analytics"),
                ("SAYA_DB_TYPE", "snowflake"),
                ("SAYA_DB_ACCOUNT", "env-account"),
                ("SAYA_DB_AUTH_TYPE", "keypair"),
                ("SAYA_DB_PRIVATE_KEY", "private-key-sentinel"),
            ])
            .with_process_env([
                ("SAYA_DB_ACCOUNT", "process-account"),
                ("SAYA_DB_USER", "env-user"),
                ("SAYA_DB_PRIVATE_KEY_PASSPHRASE", "passphrase-sentinel"),
            ]),
    )
    .unwrap();

    assert!(matches!(
        resolved.profile,
        Some(DatabaseProfile::Snowflake {
            ref account,
            ref user,
            auth_type: saya_types::SnowflakeAuth::Keypair,
            private_key: Some(SecretRef::Env { ref env }),
            passphrase: Some(SecretRef::Env { env: ref env_pass }),
            ref warehouse,
            ..
        }) if account == "process-account"
            && user == "env-user"
            && env == "SAYA_DB_PRIVATE_KEY"
            && env_pass == "SAYA_DB_PRIVATE_KEY_PASSPHRASE"
            && warehouse.as_deref() == Some("profile_wh")
    ));
    let diagnostic = format!("{resolved:?}");
    assert!(!diagnostic.contains("private-key-sentinel"));
    assert!(!diagnostic.contains("passphrase-sentinel"));
}

#[test]
fn snowflake_auth_type_transitions_clear_irrelevant_secret_refs() {
    let profiles = ConnectionsFile::from_toml(
        "[profiles.keypair]\ntype = 'snowflake'\naccount = 'account'\nuser = 'reader'\nauth_type = 'keypair'\nprivate_key = { env = 'PROFILE_KEY' }\npassphrase = { env = 'PROFILE_PASSPHRASE' }\npassword = { env = 'PROFILE_PASSWORD' }\n\n[profiles.userpass]\ntype = 'snowflake'\naccount = 'account'\nuser = 'reader'\nauth_type = 'userpass'\nprivate_key = { env = 'PROFILE_KEY' }\npassphrase = { env = 'PROFILE_PASSPHRASE' }\npassword = { env = 'PROFILE_PASSWORD' }\n\n[profiles.browser]\ntype = 'snowflake'\naccount = 'account'\nuser = 'reader'\nauth_type = 'externalbrowser'\nprivate_key = { env = 'PROFILE_KEY' }\npassphrase = { env = 'PROFILE_PASSPHRASE' }\npassword = { env = 'PROFILE_PASSWORD' }\n",
    )
    .unwrap();

    let keypair = resolve(ResolutionInput::new(profiles.clone()).with_process_env([
        ("SAYA_PROFILE", "keypair"),
        ("SAYA_DB_AUTH_TYPE", "keypair"),
    ]))
    .unwrap();
    assert!(matches!(
        keypair.profile,
        Some(DatabaseProfile::Snowflake {
            password: None,
            private_key: Some(_),
            passphrase: Some(_),
            ..
        })
    ));

    let userpass = resolve(ResolutionInput::new(profiles.clone()).with_process_env([
        ("SAYA_PROFILE", "userpass"),
        ("SAYA_DB_AUTH_TYPE", "userpass"),
    ]))
    .unwrap();
    assert!(matches!(
        userpass.profile,
        Some(DatabaseProfile::Snowflake {
            private_key: None,
            passphrase: None,
            password: Some(_),
            ..
        })
    ));

    let browser = resolve(ResolutionInput::new(profiles).with_process_env([
        ("SAYA_PROFILE", "browser"),
        ("SAYA_DB_AUTH_TYPE", "externalbrowser"),
    ]))
    .unwrap();
    assert!(matches!(
        browser.profile,
        Some(DatabaseProfile::Snowflake {
            private_key: None,
            passphrase: None,
            password: None,
            ..
        })
    ));
}

#[test]
fn snowflake_environment_overlay_validates_auth_specific_fields() {
    let missing_account = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_DB_TYPE", "snowflake"),
            ("SAYA_DB_USER", "reader"),
            ("SAYA_DB_AUTH_TYPE", "externalbrowser"),
        ]),
    )
    .unwrap_err();
    assert!(matches!(
        missing_account,
        ConfigError::MissingDatabaseField {
            name: "SAYA_DB_ACCOUNT"
        }
    ));

    let missing_key = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_DB_TYPE", "snowflake"),
            ("SAYA_DB_ACCOUNT", "account"),
            ("SAYA_DB_USER", "reader"),
            ("SAYA_DB_AUTH_TYPE", "keypair"),
        ]),
    )
    .unwrap_err();
    assert!(matches!(
        missing_key,
        ConfigError::MissingDatabaseField {
            name: "SAYA_DB_PRIVATE_KEY"
        }
    ));

    let invalid_auth = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_DB_TYPE", "snowflake"),
            ("SAYA_DB_ACCOUNT", "account"),
            ("SAYA_DB_USER", "reader"),
            ("SAYA_DB_AUTH_TYPE", "password"),
        ]),
    )
    .unwrap_err();
    assert!(matches!(
        invalid_auth,
        ConfigError::InvalidEnvironment { ref name, .. } if name == "SAYA_DB_AUTH_TYPE"
    ));

    let missing_auth = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_DB_TYPE", "snowflake"),
            ("SAYA_DB_ACCOUNT", "account"),
            ("SAYA_DB_USER", "reader"),
        ]),
    )
    .unwrap_err();
    assert!(matches!(
        missing_auth,
        ConfigError::MissingDatabaseField {
            name: "SAYA_DB_AUTH_TYPE"
        }
    ));

    let missing_password = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_DB_TYPE", "snowflake"),
            ("SAYA_DB_ACCOUNT", "account"),
            ("SAYA_DB_USER", "reader"),
            ("SAYA_DB_AUTH_TYPE", "userpass"),
        ]),
    )
    .unwrap_err();
    assert!(matches!(
        missing_password,
        ConfigError::MissingDatabaseField {
            name: "SAYA_DB_PASSWORD"
        }
    ));
}

#[test]
fn snowflake_externalbrowser_environment_needs_no_secret() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default()).with_process_env([
            ("SAYA_DB_TYPE", "snowflake"),
            ("SAYA_DB_ACCOUNT", "account"),
            ("SAYA_DB_USER", "reader"),
            ("SAYA_DB_AUTH_TYPE", "externalbrowser"),
        ]),
    )
    .unwrap();
    assert!(matches!(
        resolved.profile,
        Some(DatabaseProfile::Snowflake {
            auth_type: saya_types::SnowflakeAuth::Externalbrowser,
            private_key: None,
            password: None,
            passphrase: None,
            ..
        })
    ));
}
