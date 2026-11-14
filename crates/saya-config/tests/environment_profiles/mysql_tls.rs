use super::*;

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
