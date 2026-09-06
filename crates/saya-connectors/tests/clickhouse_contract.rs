use saya_config::MapSecretResolver;
use saya_connectors::{ConnectorOptions, build_connector, prepare_clickhouse_sql};
use saya_types::{ConnectionError, DatabaseProfile, SecretRef};

/// Destructive verbs the ClickHouse dialect admits — some parse to a statement
/// the allow-list rejects, others fail to parse at all. Both paths must end in
/// a rejection; the table below was built by probing each verb against the
/// sqlparser ClickHouse dialect and is re-checked here so a parser upgrade
/// cannot silently start accepting one.
const REJECTED_DESTRUCTIVE: &[&str] = &[
    "ALTER TABLE t DELETE WHERE x = 1",
    "ALTER TABLE t UPDATE x = 1 WHERE y = 2",
    "ALTER TABLE t ADD COLUMN c Int8",
    "ALTER TABLE t DROP COLUMN c",
    "ALTER TABLE t MODIFY COLUMN c String",
    "ALTER TABLE t RENAME COLUMN a TO b",
    "TRUNCATE TABLE t",
    "DROP TABLE t",
    "DROP DATABASE db",
    "CREATE TABLE t (x Int8)",
    "CREATE DATABASE db",
    "INSERT INTO t VALUES (1)",
    "INSERT INTO t SELECT * FROM s",
    "UPDATE t SET x = 1 WHERE y = 2",
    "DELETE FROM t WHERE x = 1",
    "OPTIMIZE TABLE t FINAL",
    "OPTIMIZE TABLE t",
    "SYSTEM RELOAD CONFIG",
    "SYSTEM FLUSH LOGS",
    "SYSTEM KILL QUERY",
    "KILL QUERY WHERE query_id = 'x'",
    "SET max_threads = 1",
    "DETACH TABLE t",
    "ATTACH TABLE t",
    "RENAME TABLE a TO b",
    "GRANT SELECT ON t TO u",
    "REVOKE SELECT ON t FROM u",
    "CHECK TABLE t",
];

#[test]
fn safety_rejects_every_clickhouse_destructive_verb() {
    for sql in REJECTED_DESTRUCTIVE {
        assert!(
            prepare_clickhouse_sql(sql, 10).is_err(),
            "must reject destructive verb: {sql}"
        );
        // A zero row cap is rejected before the statement is even examined.
        assert!(
            prepare_clickhouse_sql(sql, 0).is_err(),
            "must reject zero-cap destructive verb: {sql}"
        );
    }
}

/// Table functions that read from an arbitrary URL, object store, remote
/// server, the local filesystem, or another database. Each is an SSRF or
/// local-file-read vector that a read-only session must not open; the guard
/// reaches them wherever they appear (here, in `FROM` with call arguments).
const REJECTED_EXTERNAL: &[&str] = &[
    "SELECT * FROM url('http://x/y.csv', CSV)",
    "SELECT * FROM s3('bucket/key')",
    "SELECT * FROM remote('host:9000', 'db', 't')",
    "SELECT * FROM mysql('host:3306', 'db', 't')",
    "SELECT * FROM postgresql('host:5432', 'db', 't')",
    "SELECT * FROM file('data.csv')",
    "SELECT * FROM hdfs('uri')",
    "SELECT * FROM odbc('dsn')",
    "SELECT * FROM jdbc('url')",
];

#[test]
fn safety_rejects_clickhouse_external_table_functions() {
    for sql in REJECTED_EXTERNAL {
        assert!(
            prepare_clickhouse_sql(sql, 10).is_err(),
            "must reject external table function: {sql}"
        );
    }
    // A safe built-in table function stays available.
    assert!(prepare_clickhouse_sql("SELECT * FROM numbers(10)", 10).is_ok());
}

#[test]
fn safety_rejects_format_clause_so_the_connector_owns_wire_format() {
    for sql in [
        "SELECT * FROM t FORMAT JSONEachRow",
        "SELECT 1 FORMAT CSV",
        "EXPLAIN SELECT 1 FORMAT TSV",
    ] {
        assert!(
            prepare_clickhouse_sql(sql, 10).is_err(),
            "must reject FORMAT clause: {sql}"
        );
    }
}

#[test]
fn safety_accepts_and_caps_clickhouse_reads() {
    for sql in [
        "SELECT * FROM t",
        "SELECT * FROM system.tables",
        "WITH x AS (SELECT 1) SELECT * FROM x",
        "SHOW TABLES",
        "EXPLAIN SELECT 1",
        "EXPLAIN ANALYZE SELECT 1",
    ] {
        assert!(
            prepare_clickhouse_sql(sql, 10).is_ok(),
            "must accept read: {sql}"
        );
    }
    // No explicit limit: the cap is injected one above the row cap, matching
    // the other connectors.
    assert!(
        prepare_clickhouse_sql("SELECT * FROM t", 10)
            .unwrap()
            .to_uppercase()
            .contains("LIMIT 11"),
    );
    // An explicit limit at or under the cap is left as written.
    assert!(
        prepare_clickhouse_sql("SELECT 1 FROM t LIMIT 5", 10)
            .unwrap()
            .to_uppercase()
            .contains("LIMIT 5"),
    );
}

#[test]
fn safety_rejections_name_the_reason() {
    let cases = [
        ("DELETE FROM t", "DELETE"),
        ("ALTER TABLE t ADD COLUMN c Int8", "ALTER TABLE"),
        ("OPTIMIZE TABLE t", "this statement"),
        ("SELECT * FROM url('http://x')", "url"),
        ("SELECT * FROM t FORMAT JSONEachRow", "FORMAT"),
    ];
    for (sql, expected) in cases {
        let error = prepare_clickhouse_sql(sql, 10)
            .expect_err("must reject")
            .to_string();
        assert!(
            error.contains("rejected") && error.contains(expected),
            "rejection must explain itself ({expected}): {error}"
        );
    }
}

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
