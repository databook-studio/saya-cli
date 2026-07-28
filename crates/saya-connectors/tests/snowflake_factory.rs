use saya_config::MapSecretResolver;
use saya_connectors::{ConnectorOptions, build_connector};
use saya_types::{DatabaseProfile, SecretRef, SnowflakeAuth};

fn profile(account: &str, auth_type: SnowflakeAuth) -> DatabaseProfile {
    DatabaseProfile::Snowflake {
        account: account.into(),
        user: "user".into(),
        auth_type,
        private_key: Some(SecretRef::Env { env: "key".into() }),
        password: Some(SecretRef::Env {
            env: "password".into(),
        }),
        passphrase: None,
        warehouse: None,
        database: None,
        schema: None,
        role: None,
    }
}

#[tokio::test]
async fn factory_requires_each_auth_secret_and_never_leaks_resolved_values() {
    let key = profile("xy12345.ap-southeast-2.aws", SnowflakeAuth::Keypair);
    let missing_key = MapSecretResolver::new([]);
    let error = build_connector(&key, &missing_key, ConnectorOptions::default())
        .await
        .err()
        .unwrap();
    assert!(!error.to_string().contains("key-sentinel"));
    let mut no_key = profile("xy12345", SnowflakeAuth::Keypair);
    if let DatabaseProfile::Snowflake { private_key, .. } = &mut no_key {
        *private_key = None;
    }
    assert!(
        build_connector(
            &no_key,
            &MapSecretResolver::new([]),
            ConnectorOptions::default()
        )
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("required")
    );
    let mut no_password = profile("xy12345", SnowflakeAuth::Userpass);
    if let DatabaseProfile::Snowflake { password, .. } = &mut no_password {
        *password = None;
    }
    assert!(
        build_connector(
            &no_password,
            &MapSecretResolver::new([]),
            ConnectorOptions::default()
        )
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("required")
    );
    let userpass = profile("bad/account", SnowflakeAuth::Userpass);
    let error = build_connector(
        &userpass,
        &MapSecretResolver::new([("password".into(), "password-sentinel".into())]),
        ConnectorOptions::default(),
    )
    .await
    .err()
    .unwrap();
    assert!(!error.to_string().contains("password-sentinel"));
}

#[tokio::test]
async fn factory_accepts_valid_account_forms_and_rejects_host_injection() {
    let secrets = MapSecretResolver::new([("key".into(), "not-parsed-until-auth".into())]);
    for account in [
        "xy12345",
        "org-account.us-east-1.aws",
        "myorg.eu-central-1.azure",
    ] {
        assert!(
            build_connector(
                &profile(account, SnowflakeAuth::Keypair),
                &secrets,
                ConnectorOptions::default()
            )
            .await
            .is_ok(),
            "{account}"
        );
    }
    for account in [
        "https://xy12345",
        "xy12345/path",
        "xy12345?query",
        "xy@host",
        "xy..host",
        "-bad",
        "bad-",
        "good.-bad.aws",
        "",
    ] {
        assert!(
            build_connector(
                &profile(account, SnowflakeAuth::Keypair),
                &secrets,
                ConnectorOptions::default()
            )
            .await
            .is_err(),
            "{account}"
        );
    }
}

#[tokio::test]
async fn external_browser_profile_fails_closed() {
    let profile = profile("xy12345", SnowflakeAuth::Externalbrowser);
    let connector = build_connector(
        &profile,
        &MapSecretResolver::new([]),
        ConnectorOptions::default(),
    )
    .await
    .unwrap();
    assert!(
        connector
            .connect()
            .await
            .unwrap_err()
            .to_string()
            .contains("interactive mode")
    );
}
