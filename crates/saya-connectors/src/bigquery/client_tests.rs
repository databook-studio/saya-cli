use super::*;
use rsa::RsaPrivateKey;
use rsa::pkcs8::{EncodePrivateKey, LineEnding};

fn key_json(token_uri: &str) -> String {
    let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
    let private_key = key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
    format!(
        r#"{{"client_email":"reader@proj.iam.gserviceaccount.com","private_key":{private_key:?},"token_uri":{token_uri:?}}}"#
    )
}

fn connector() -> BigQueryConnector {
    BigQueryConnector::new(
        "my-project".into(),
        Some("analytics".into()),
        None,
        None,
        key_json("https://oauth2.googleapis.com/token"),
        ConnectorOptions::default(),
    )
    .unwrap()
}

#[test]
fn construction_applies_default_byte_cap() {
    let connector = connector();
    assert_eq!(connector.max_bytes_billed, DEFAULT_MAX_BYTES_BILLED);
    assert_eq!(
        connector.query_url(),
        "https://bigquery.googleapis.com/bigquery/v2/projects/my-project/queries"
    );
    assert_eq!(
        connector.jobs_url(),
        "https://bigquery.googleapis.com/bigquery/v2/projects/my-project/jobs"
    );
}

#[test]
fn custom_byte_cap_is_honored() {
    let connector = BigQueryConnector::new(
        "p".into(),
        None,
        None,
        Some(1024),
        key_json("https://oauth2.googleapis.com/token"),
        ConnectorOptions::default(),
    )
    .unwrap();
    assert_eq!(connector.max_bytes_billed, 1024);
}

#[test]
fn invalid_project_is_rejected_before_any_request() {
    let result = BigQueryConnector::new(
        "proj/path".into(),
        None,
        None,
        None,
        key_json("https://oauth2.googleapis.com/token"),
        ConnectorOptions::default(),
    );
    let error = match result {
        Ok(_) => panic!("an invalid project must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(error, ConnectionError::InvalidConfiguration(_)));
}

/// BigQuery binds parameters natively through its `queryParameters`.
#[test]
fn bigquery_supports_parameters() {
    assert!(connector().supports_parameters());
}

#[test]
fn malformed_key_is_rejected_as_configuration() {
    let result = BigQueryConnector::new(
        "p".into(),
        None,
        None,
        None,
        "not a key".into(),
        ConnectorOptions::default(),
    );
    assert!(matches!(
        result,
        Err(ConnectionError::InvalidConfiguration(_))
    ));
}
