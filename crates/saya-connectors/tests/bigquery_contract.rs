use saya_config::MapSecretResolver;
use saya_connectors::{ConnectorOptions, build_connector, prepare_bigquery_sql};
use saya_types::{ConnectionError, DatabaseProfile, SecretRef};

use rsa::pkcs8::EncodePrivateKey;

/// Destructive verbs BigQuery admits — DML, DDL, and scripting. Some parse to
/// a statement the allow-list rejects; others fail to parse at all. Both paths
/// must end in a rejection, and each is pinned here so a parser upgrade cannot
/// silently start accepting one.
const REJECTED_DESTRUCTIVE: &[&str] = &[
    "INSERT INTO `p.d.t` (x) VALUES (1)",
    "INSERT INTO `p.d.t` SELECT * FROM `p.d.s`",
    "UPDATE `p.d.t` SET x = 1 WHERE y = 2",
    "DELETE FROM `p.d.t` WHERE x = 1",
    "MERGE `p.d.t` USING `p.d.s` ON t.x = s.x WHEN MATCHED THEN DELETE",
    "CREATE TABLE `p.d.t` (x INT64)",
    "CREATE VIEW `p.d.v` AS SELECT 1",
    "CREATE SCHEMA `p.d`",
    "DROP TABLE `p.d.t`",
    "DROP SCHEMA `p.d`",
    "ALTER TABLE `p.d.t` ADD COLUMN c INT64",
    "TRUNCATE TABLE `p.d.t`",
    "BEGIN SELECT 1; END",
    "CALL `p.d.sp`()`",
    "EXPORT DATA OPTIONS(uri='gs://bucket/out') OVERWRITE `p.d.t`",
    "GRANT roles/viewer ON `p.d.t` TO 'user@example.com'",
    "REVOKE roles/viewer ON `p.d.t` FROM 'user@example.com'",
];

#[test]
fn safety_rejects_every_bigquery_destructive_verb() {
    for sql in REJECTED_DESTRUCTIVE {
        assert!(
            prepare_bigquery_sql(sql, 10).is_err(),
            "must reject destructive verb: {sql}"
        );
        // A zero row cap is rejected before the statement is even examined.
        assert!(
            prepare_bigquery_sql(sql, 0).is_err(),
            "must reject zero-cap destructive verb: {sql}"
        );
    }
}

/// `EXTERNAL_QUERY` runs a query against an external Cloud SQL database over a
/// federated connection — a read-only session has no business opening another
/// database, so it is denied wherever it appears.
#[test]
fn safety_rejects_external_query() {
    assert!(
        prepare_bigquery_sql(
            "SELECT * FROM EXTERNAL_QUERY(\"proj.region.conn\", \"SELECT * FROM users\")",
            10
        )
        .is_err(),
        "EXTERNAL_QUERY must be rejected"
    );
}

#[test]
fn safety_accepts_and_caps_bigquery_reads() {
    for sql in [
        "SELECT * FROM `p.d.t`",
        "WITH x AS (SELECT 1) SELECT * FROM x",
        "SELECT 1 FROM `p.d.t` UNION ALL SELECT 2 FROM `p.d.s`",
        "EXPLAIN SELECT 1",
        "EXPLAIN ANALYZE SELECT 1",
    ] {
        assert!(
            prepare_bigquery_sql(sql, 10).is_ok(),
            "must accept read: {sql}"
        );
    }
    // No explicit limit: the cap is injected one above the row cap.
    assert!(
        prepare_bigquery_sql("SELECT * FROM `p.d.t`", 10)
            .unwrap()
            .to_uppercase()
            .contains("LIMIT 11"),
    );
    // An explicit limit at or under the cap is left as written.
    assert!(
        prepare_bigquery_sql("SELECT 1 FROM `p.d.t` LIMIT 5", 10)
            .unwrap()
            .to_uppercase()
            .contains("LIMIT 5"),
    );
}

#[test]
fn safety_rejections_name_the_reason() {
    let cases = [
        ("DELETE FROM `p.d.t`", "DELETE"),
        ("CREATE TABLE `p.d.t` (x INT64)", "CREATE"),
        ("DROP TABLE `p.d.t`", "DROP"),
        ("ALTER TABLE `p.d.t` ADD COLUMN c INT64", "ALTER TABLE"),
        (
            "SELECT * FROM EXTERNAL_QUERY(\"c\", \"SELECT 1\")",
            "EXTERNAL_QUERY",
        ),
    ];
    for (sql, expected) in cases {
        let error = prepare_bigquery_sql(sql, 10)
            .expect_err("must reject")
            .to_string();
        assert!(
            error.contains("rejected") && error.contains(expected),
            "rejection must explain itself ({expected}): {error}"
        );
    }
}

fn key_json() -> String {
    let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
    let private_key = key
        .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
        .unwrap()
        .to_string();
    format!(
        r#"{{"client_email":"reader@proj.iam.gserviceaccount.com","private_key":{private_key:?},"token_uri":"https://oauth2.googleapis.com/token"}}"#
    )
}

fn bigquery(key: Option<SecretRef>) -> DatabaseProfile {
    DatabaseProfile::BigQuery {
        project: "my-project".into(),
        dataset: Some("analytics".into()),
        location: None,
        max_bytes_billed: None,
        service_account_key: key.unwrap_or_else(|| SecretRef::Env {
            env: "SAYA_BIGQUERY_KEY".into(),
        }),
    }
}

#[tokio::test]
async fn factory_builds_bigquery_with_dialect_and_unsupported_cancel() {
    // Build does not connect, so a server need not be present.
    let connector = build_connector(
        &bigquery(None),
        &MapSecretResolver::new([("SAYA_BIGQUERY_KEY".into(), key_json())]),
        ConnectorOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(connector.dialect().as_str(), "bigquery");
    // Cancellation is not implemented over the REST transport; the default
    // contract surfaces that honestly rather than silently no-op-ing.
    assert!(matches!(
        connector.cancel().await,
        Err(ConnectionError::Unsupported(_))
    ));
}

#[tokio::test]
async fn factory_missing_secret_fails_configuration_without_leaking() {
    let result = build_connector(
        &bigquery(None),
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

#[tokio::test]
async fn factory_malformed_key_fails_without_leaking_the_resolved_secret() {
    // The resolver hands back a planted value that is not a valid key; the
    // build must fail at configuration, and the planted value must not survive
    // into the error message. `Box<dyn DatabaseConnector>` is not `Debug` (it
    // holds resolved secrets), so the error is read by matching.
    let sentinel = "not-a-valid-key-sentinel";
    let result = build_connector(
        &bigquery(None),
        &MapSecretResolver::new([("SAYA_BIGQUERY_KEY".into(), sentinel.into())]),
        ConnectorOptions::default(),
    )
    .await;
    let error = match result {
        Ok(_) => panic!("a malformed key must fail to build"),
        Err(error) => error,
    };
    assert!(matches!(error, ConnectionError::InvalidConfiguration(_)));
    assert!(!error.to_string().contains(sentinel));
}
