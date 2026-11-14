use super::*;

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
