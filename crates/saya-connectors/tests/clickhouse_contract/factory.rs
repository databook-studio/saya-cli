use saya_config::MapSecretResolver;
use saya_connectors::{ConnectorOptions, build_connector};
use saya_types::{ConnectionError, DatabaseProfile, SecretRef};

fn clickhouse(host: &str, port: Option<u16>, password: Option<SecretRef>) -> DatabaseProfile {
    DatabaseProfile::ClickHouse {
        host: host.into(),
        port,
        database: Some("analytics".into()),
        user: Some("reader".into()),
        password,
        secure: None,
    }
}

#[tokio::test]
async fn factory_builds_clickhouse_with_dialect_and_unsupported_cancel() {
    // Port 1 on loopback is closed, so the build succeeds (no connection is
    // made) and the dialect is reported without a server present.
    let connector = build_connector(
        &clickhouse("127.0.0.1", Some(1), None),
        &MapSecretResolver::new([]),
        ConnectorOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(connector.dialect().as_str(), "clickhouse");
    // Cancellation is not implemented for the HTTP transport; the default
    // contract surfaces that honestly rather than silently no-op-ing.
    assert!(matches!(
        connector.cancel().await,
        Err(ConnectionError::Unsupported(_))
    ));
}

#[tokio::test]
async fn factory_connection_error_does_not_expose_the_password() {
    let connector = build_connector(
        &clickhouse(
            "127.0.0.1",
            Some(1),
            Some(SecretRef::Env {
                env: "CLICKHOUSE_SENTINEL".into(),
            }),
        ),
        &MapSecretResolver::new([(
            "CLICKHOUSE_SENTINEL".into(),
            "clickhouse-secret-sentinel".into(),
        )]),
        ConnectorOptions {
            query_timeout_seconds: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let error = connector
        .connect()
        .await
        .expect_err("closed port must fail");
    assert!(!error.to_string().contains("clickhouse-secret-sentinel"));
    assert!(matches!(error, ConnectionError::ConnectionFailed(_)));
}

#[tokio::test]
async fn factory_missing_secret_fails_configuration_without_leaking() {
    // `Box<dyn DatabaseConnector>` is not `Debug` (it holds resolved secrets),
    // so the error is read by matching rather than by `expect_err`.
    let result = build_connector(
        &clickhouse(
            "127.0.0.1",
            Some(1),
            Some(SecretRef::Env {
                env: "CLICKHOUSE_MISSING".into(),
            }),
        ),
        &MapSecretResolver::new([]),
        ConnectorOptions::default(),
    )
    .await;
    let error = match result {
        Ok(_) => panic!("a missing secret must fail to build"),
        Err(error) => error,
    };
    assert!(matches!(error, ConnectionError::InvalidConfiguration(_)));
}

/// Basic auth carries the password in a header, so plain HTTP to a remote host
/// puts it on the network. A local server is the case ClickHouse's own default
/// port is for and stays allowed; anywhere else has to ask for TLS explicitly
/// rather than be downgraded without saying so.
#[tokio::test]
async fn a_password_over_plain_http_is_refused_for_a_remote_host() {
    let secret = || {
        Some(SecretRef::Env {
            env: "CLICKHOUSE_TLS_CHECK".into(),
        })
    };
    let resolver =
        || MapSecretResolver::new([("CLICKHOUSE_TLS_CHECK".into(), "a-password".into())]);

    let remote = build_connector(
        &clickhouse("warehouse.example.com", None, secret()),
        &resolver(),
        ConnectorOptions::default(),
    )
    .await;
    assert!(
        remote.is_err(),
        "a remote host with a password and no TLS must be refused"
    );

    let local = build_connector(
        &clickhouse("127.0.0.1", None, secret()),
        &resolver(),
        ConnectorOptions::default(),
    )
    .await;
    assert!(
        local.is_ok(),
        "a local server keeps the password off the network"
    );

    let mut secured = clickhouse("warehouse.example.com", None, secret());
    if let DatabaseProfile::ClickHouse { secure, .. } = &mut secured {
        *secure = Some(true);
    }
    assert!(
        build_connector(&secured, &resolver(), ConnectorOptions::default())
            .await
            .is_ok(),
        "TLS makes the remote host acceptable"
    );
}
