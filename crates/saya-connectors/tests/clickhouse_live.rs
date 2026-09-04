//! Live ClickHouse exercises. These need a reachable server and run only when
//! `SAYA_TEST_CLICKHOUSE_HOST` is set. Setup DDL is issued through a plain HTTP
//! client because the connector itself is read-only and would reject it.
//!
//! Example:
//!   SAYA_TEST_CLICKHOUSE_HOST=127.0.0.1 SAYA_TEST_CLICKHOUSE_USER=default \
//!     SAYA_TEST_CLICKHOUSE_DATABASE=saya_live cargo test --test clickhouse_live

use saya_config::MapSecretResolver;
use saya_connectors::{ConnectorOptions, build_connector};
use saya_types::{DatabaseProfile, QueryRequest, SecretRef};

fn live_profile() -> Option<(DatabaseProfile, String, Option<String>)> {
    let host = std::env::var("SAYA_TEST_CLICKHOUSE_HOST").ok()?;
    let secure = std::env::var("SAYA_TEST_CLICKHOUSE_SECURE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let port = std::env::var("SAYA_TEST_CLICKHOUSE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(if secure { 8443 } else { 8123 });
    let user = std::env::var("SAYA_TEST_CLICKHOUSE_USER").ok();
    let password_value = std::env::var("SAYA_TEST_CLICKHOUSE_PASSWORD").ok();
    let password = password_value.as_ref().map(|_| SecretRef::Env {
        env: "SAYA_TEST_CLICKHOUSE_PASSWORD".into(),
    });
    let database = std::env::var("SAYA_TEST_CLICKHOUSE_DATABASE").ok();
    let scheme = if secure { "https" } else { "http" };
    let endpoint = format!("{scheme}://{host}:{port}/");
    let profile = DatabaseProfile::ClickHouse {
        host,
        port: Some(port),
        database,
        user,
        password,
        secure: Some(secure),
    };
    Some((profile, endpoint, password_value))
}

/// Runs arbitrary SQL for fixture setup, bypassing the read-only connector.
async fn admin(endpoint: &str, user: &Option<String>, password: &Option<String>, sql: &str) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let mut request = client.post(endpoint).body(sql.to_string());
    if let Some(user) = user {
        request = request.basic_auth(user, password.as_deref());
    }
    let response = request.send().await.unwrap();
    assert!(
        response.status().is_success(),
        "admin setup failed for `{sql}`: HTTP {}",
        response.status()
    );
}

#[tokio::test]
async fn live_clickhouse_executes_discovers_schema_and_rejects_writes() {
    let Some((profile, endpoint, password_value)) = live_profile() else {
        return;
    };
    let user = match &profile {
        DatabaseProfile::ClickHouse { user, .. } => user.clone(),
        _ => unreachable!(),
    };
    let mut resolver_map = Vec::new();
    if let Some(value) = &password_value {
        resolver_map.push(("SAYA_TEST_CLICKHOUSE_PASSWORD".into(), value.clone()));
    }
    let connector = build_connector(
        &profile,
        &MapSecretResolver::new(resolver_map),
        ConnectorOptions::default(),
    )
    .await
    .unwrap();
    connector.connect().await.unwrap();

    let table = "saya_ch_live_fixture";
    admin(
        &endpoint,
        &user,
        &password_value,
        &format!("DROP TABLE IF EXISTS {table}"),
    )
    .await;
    admin(
        &endpoint,
        &user,
        &password_value,
        &format!(
            "CREATE TABLE {table} (id UInt64, name String, maybe_val Nullable(String)) \
             ENGINE = MergeTree ORDER BY id"
        ),
    )
    .await;
    admin(
        &endpoint,
        &user,
        &password_value,
        &format!("INSERT INTO {table} (id, name, maybe_val) VALUES (1, 'a', NULL), (2, 'b', 'x')"),
    )
    .await;

    let result = connector
        .execute(QueryRequest::new(
            format!("SELECT id, name, maybe_val FROM {table} ORDER BY id"),
            1,
        ))
        .await
        .unwrap();
    assert_eq!(result.columns, vec!["id", "name", "maybe_val"]);
    assert_eq!(result.row_count, 1);
    assert!(result.truncated);
    assert_eq!(result.rows[0][0], 1);
    assert_eq!(result.rows[0][1], "a");
    assert!(result.rows[0][2].is_null());

    let schema = connector.schema().await.unwrap();
    let fixture = schema
        .databases
        .iter()
        .flat_map(|database| &database.schemas)
        .flat_map(|schema| &schema.tables)
        .find(|table| table.name == "saya_ch_live_fixture")
        .expect("fixture table must be discovered");
    let columns: Vec<&str> = fixture.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(columns, vec!["id", "name", "maybe_val"]);
    // ClickHouse has no foreign keys; none are invented.
    assert!(fixture.foreign_keys.is_empty());

    // A destructive statement is rejected by the safety layer before it reaches
    // the server, and the rejection does not echo the password.
    let rejected = connector
        .execute(QueryRequest::new(format!("DROP TABLE {table}"), 1))
        .await
        .expect_err("DROP must be rejected");
    assert!(
        !rejected
            .to_string()
            .contains(password_value.as_deref().unwrap_or(""))
    );

    admin(
        &endpoint,
        &user,
        &password_value,
        &format!("DROP TABLE {table}"),
    )
    .await;
}
