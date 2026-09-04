use saya_config::MapSecretResolver;
use saya_connectors::{ConnectorOptions, build_connector};
use saya_types::{DatabaseProfile, SecretRef};

/// Live BigQuery contract — opt-in via environment variables. No credentials
/// are required to build the workspace; this test skips cleanly when the key
/// or project is unset.
///
/// Required:
///   SAYA_TEST_BIGQUERY_PROJECT — the GCP project id to query.
///   SAYA_TEST_BIGQUERY_KEY     — the service-account JSON key file contents.
/// Optional:
///   SAYA_TEST_BIGQUERY_DATASET — when set, schema discovery is exercised.
///   SAYA_TEST_BIGQUERY_LOCATION — job location such as US or EU.
///   SAYA_TEST_BIGQUERY_MAX_BYTES — overrides the per-job byte cap.
#[tokio::test]
async fn bigquery_live_contract_is_opt_in() {
    let Ok(project) = std::env::var("SAYA_TEST_BIGQUERY_PROJECT") else {
        eprintln!("SKIPPED: SAYA_TEST_BIGQUERY_PROJECT is unset");
        return;
    };
    let Ok(key) = std::env::var("SAYA_TEST_BIGQUERY_KEY") else {
        eprintln!("SKIPPED: SAYA_TEST_BIGQUERY_KEY is unset");
        return;
    };
    let dataset = std::env::var("SAYA_TEST_BIGQUERY_DATASET").ok();
    let location = std::env::var("SAYA_TEST_BIGQUERY_LOCATION").ok();
    let max_bytes_billed = std::env::var("SAYA_TEST_BIGQUERY_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let profile = DatabaseProfile::BigQuery {
        project,
        dataset: dataset.clone(),
        location,
        max_bytes_billed,
        service_account_key: SecretRef::Env {
            env: "SAYA_TEST_BIGQUERY_KEY".into(),
        },
    };
    let resolver = MapSecretResolver::new([("SAYA_TEST_BIGQUERY_KEY".into(), key)]);
    let connector = build_connector(&profile, &resolver, ConnectorOptions::default())
        .await
        .unwrap();
    connector.connect().await.unwrap();
    connector
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap();
    if dataset.is_some() {
        connector.schema().await.unwrap();
    }
}
