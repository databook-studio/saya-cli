use saya_config::MapSecretResolver;
use saya_connectors::{ConnectorOptions, build_connector};
use saya_types::{DatabaseProfile, MySqlSslMode};

fn mysql() -> DatabaseProfile {
    DatabaseProfile::Mysql {
        host: "127.0.0.1".into(),
        port: Some(3306),
        database: "warehouse".into(),
        user: "reader".into(),
        ssl_mode: Some(MySqlSslMode::Require),
        ssl_ca: None,
        password: None,
    }
}

#[tokio::test]
async fn factory_builds_mysql_without_a_credential_url() {
    let connector = build_connector(
        &mysql(),
        &MapSecretResolver::new([]),
        ConnectorOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(connector.dialect().as_str(), "mysql");
    // Idle cancellation is a no-op, not an unsupported error: MySQL queries
    // are cancellable via KILL QUERY once one is in flight.
    assert!(connector.cancel().await.is_ok());
}

#[tokio::test]
async fn mysql_secret_sentinels_are_not_exposed_by_factory_errors() {
    let mut profile = mysql();
    if let DatabaseProfile::Mysql { password, .. } = &mut profile {
        *password = Some(saya_types::SecretRef::Env {
            env: "MYSQL_SENTINEL".into(),
        });
    }
    let connector = build_connector(
        &profile,
        &MapSecretResolver::new([("MYSQL_SENTINEL".into(), "mysql-secret-sentinel".into())]),
        ConnectorOptions {
            query_timeout_seconds: 1,
            max_connections: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let error = connector.connect().await.unwrap_err();
    assert!(!error.to_string().contains("mysql-secret-sentinel"));
}
