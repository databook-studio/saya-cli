use std::{sync::Arc, time::Duration};

use saya_config::MapSecretResolver;
use saya_connectors::{ConnectorOptions, DatabaseConnector, DuckDbConnector, build_connector};
use saya_types::{DatabaseProfile, MySqlSslMode, QueryRequest};

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
    assert!(connector.cancel().await.is_err());
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
        },
    )
    .await
    .unwrap();
    let error = connector.connect().await.unwrap_err();
    assert!(!error.to_string().contains("mysql-secret-sentinel"));
}

#[tokio::test]
async fn duckdb_file_schema_types_bounds_and_policy_are_hermetic() {
    let path = std::env::temp_dir().join(format!("saya-duckdb-{}.db", std::process::id()));
    let database = duckdb::Connection::open(&path).unwrap();
    database.execute_batch("CREATE TABLE events (id INTEGER, active BOOLEAN, note VARCHAR); INSERT INTO events VALUES (1, true, NULL), (2, false, 'two'), (3, true, 'three'); CREATE SEQUENCE ids").unwrap();
    drop(database);
    let profile = DatabaseProfile::DuckDb {
        path: path.display().to_string(),
        read_only: Some(false),
    };
    let connector = build_connector(
        &profile,
        &MapSecretResolver::new([]),
        ConnectorOptions::default(),
    )
    .await
    .unwrap();
    let schema = connector.schema().await.unwrap();
    assert!(
        schema
            .databases
            .iter()
            .flat_map(|db| &db.schemas)
            .flat_map(|schema| &schema.tables)
            .any(|table| table.name == "events")
    );
    let result = connector
        .execute(QueryRequest::new(
            "SELECT id, active, note FROM events ORDER BY id",
            2,
        ))
        .await
        .unwrap();
    assert_eq!(result.rows.len(), 2);
    assert!(result.truncated);
    assert_eq!(result.rows[0], serde_json::json!([1, true, null]));
    assert_eq!(
        result.executed_sql,
        "SELECT id, active, note FROM events ORDER BY id"
    );
    let values = connector
        .execute(QueryRequest::new(
            "SELECT DATE '2024-01-02', TIME '03:04:05', TIMESTAMP '2024-01-02 03:04:05', INTERVAL '2 months 3 days 4 seconds', [1, 2], {'a': 3}",
            1,
        ))
        .await
        .unwrap();
    assert_eq!(values.rows[0][0], "2024-01-02");
    assert_eq!(values.rows[0][1], "03:04:05");
    assert_eq!(values.rows[0][2], "2024-01-02T03:04:05+00:00");
    assert_eq!(
        values.rows[0][3],
        serde_json::json!({"months": 2, "days": 3, "nanos": 4_000_000_000_i64})
    );
    assert_eq!(values.rows[0][4], serde_json::json!([1, 2]));
    assert_eq!(values.rows[0][5], serde_json::json!({"a": 3}));
    for sql in [
        "CREATE TABLE denied (id INT)",
        "SELECT nextval('ids')",
        "SELECT * FROM read_csv('x.csv')",
    ] {
        assert!(
            connector.execute(QueryRequest::new(sql, 1)).await.is_err(),
            "must reject {sql}"
        );
    }
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn duckdb_read_only_file_and_interrupt_are_enforced() {
    let path = std::env::temp_dir().join(format!("saya-duckdb-ro-{}.db", std::process::id()));
    let fixture = duckdb::Connection::open(&path).unwrap();
    fixture
        .execute_batch("CREATE TABLE saved (id INTEGER); INSERT INTO saved VALUES (7)")
        .unwrap();
    drop(fixture);
    assert!(
        build_connector(
            &DatabaseProfile::DuckDb {
                path: path.display().to_string(),
                read_only: None
            },
            &MapSecretResolver::new([]),
            ConnectorOptions::default()
        )
        .await
        .is_err()
    );
    let readonly = build_connector(
        &DatabaseProfile::DuckDb {
            path: path.display().to_string(),
            read_only: Some(true),
        },
        &MapSecretResolver::new([]),
        ConnectorOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        readonly
            .execute(QueryRequest::new("SELECT id FROM saved", 1))
            .await
            .unwrap()
            .rows,
        vec![serde_json::json!([7])]
    );
    drop(readonly);
    let connector = Arc::new(
        DuckDbConnector::open(
            ":memory:",
            false,
            ConnectorOptions {
                query_timeout_seconds: 5,
                max_connections: 1,
            },
        )
        .await
        .unwrap(),
    );
    let running = {
        let connector = connector.clone();
        tokio::spawn(async move {
            connector
                .execute(QueryRequest::new(
                    "SELECT count(*) FROM range(10000000000) a, range(100) b",
                    1,
                ))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(25)).await;
    connector.cancel().await.unwrap();
    assert!(running.await.unwrap().is_err());
    // Cancellation awaits DuckDB's native interrupt so the same connection is safe to reuse.
    assert_eq!(
        connector
            .execute(QueryRequest::new("SELECT 1", 1))
            .await
            .unwrap()
            .rows,
        vec![serde_json::json!([1])]
    );
    let _ = std::fs::remove_file(path);
}
