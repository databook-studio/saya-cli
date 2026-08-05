use saya_config::MapSecretResolver;
use saya_connectors::{ConnectorOptions, build_connector, prepare_postgres_sql};
use saya_types::{ConnectionError, DatabaseProfile, PostgresSslMode, SecretRef};

fn postgres(password: Option<SecretRef>) -> DatabaseProfile {
    DatabaseProfile::Postgres {
        host: "db.example.test".into(),
        port: Some(5432),
        database: "warehouse".into(),
        user: "reader".into(),
        ssl_mode: Some(PostgresSslMode::Require),
        password,
    }
}

#[test]
fn safety_is_fail_closed_and_bounds_selects() {
    assert!(
        prepare_postgres_sql("SELECT * FROM events", 10)
            .unwrap()
            .contains("LIMIT 11")
    );
    assert!(prepare_postgres_sql("WITH x AS (SELECT 1) SELECT * FROM x", 10).is_ok());
    assert!(prepare_postgres_sql("SHOW search_path", 10).is_ok());
    assert!(prepare_postgres_sql("EXPLAIN SELECT 1", 10).is_ok());
    for sql in ["DELETE FROM events", "SELECT 1; SELECT 2", "not sql {{{"] {
        assert!(
            prepare_postgres_sql(sql, 10).is_err(),
            "should reject {sql}"
        );
    }
}

#[tokio::test]
async fn factory_resolves_secret_references_without_exposing_values() {
    let resolver = MapSecretResolver::new([(String::from("DB_PASSWORD"), String::from("hidden"))]);
    assert!(
        build_connector(
            &postgres(Some(SecretRef::Env {
                env: "DB_PASSWORD".into()
            })),
            &resolver,
            ConnectorOptions::default(),
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn factory_redacts_missing_secret_and_builds_duckdb() {
    let resolver = MapSecretResolver::new([]);
    let error = match build_connector(
        &postgres(Some(SecretRef::Env {
            env: "DB_PASSWORD".into(),
        })),
        &resolver,
        ConnectorOptions::default(),
    )
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("missing secret must fail"),
    };
    assert!(matches!(error, ConnectionError::InvalidConfiguration(_)));
    assert!(!error.to_string().contains("hidden"));

    let duckdb = build_connector(
        &DatabaseProfile::DuckDb {
            path: ":memory:".into(),
            read_only: Some(false),
        },
        &resolver,
        ConnectorOptions::default(),
    )
    .await;
    assert!(duckdb.is_ok());
}
