use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::{ConnectorOptions, DatabaseConnector};

use super::auth::{CachedToken, ServiceAccount, exchange, parse_service_account};
use super::{errors, execute, metadata};

/// Default per-job byte cap. BigQuery's on-demand free tier scans 1 TiB per
/// month, so this caps a single runaway query at roughly one month's free
/// allowance rather than the hundreds of terabytes an unbounded cross join can
/// reach. The cap is configurable; a cautious deployment lowers it.
const DEFAULT_MAX_BYTES_BILLED: u64 = 1 << 40;

/// Connects to BigQuery over its REST API. Every statement a caller supplies
/// is first narrowed to a read by the shared safety layer; this client only
/// carries the result back over the wire. The access token and the
/// service-account key are secrets, so this struct deliberately does not
/// implement `Debug`.
pub struct BigQueryConnector {
    pub(crate) client: Client,
    pub(crate) project: String,
    pub(crate) dataset: Option<String>,
    pub(crate) location: Option<String>,
    pub(crate) max_bytes_billed: u64,
    pub(crate) api_origin: String,
    pub(crate) service_account: ServiceAccount,
    pub(crate) token: Arc<Mutex<Option<CachedToken>>>,
    pub(crate) timeout: Duration,
}

impl BigQueryConnector {
    pub(crate) fn new(
        project: String,
        dataset: Option<String>,
        location: Option<String>,
        max_bytes_billed: Option<u64>,
        key_json: String,
        settings: ConnectorOptions,
    ) -> Result<Self, ConnectionError> {
        if !valid_project(&project) {
            return Err(ConnectionError::invalid_configuration(
                "invalid BigQuery project id",
            ));
        }
        let service_account = parse_service_account(&key_json).map_err(|_| errors::config())?;
        let timeout = Duration::from_secs(settings.query_timeout_seconds.max(1));
        let client = Client::builder()
            .connect_timeout(timeout)
            .timeout(timeout)
            .build()
            .map_err(|_| errors::build())?;
        Ok(Self {
            client,
            project,
            dataset,
            location,
            max_bytes_billed: max_bytes_billed.unwrap_or(DEFAULT_MAX_BYTES_BILLED),
            api_origin: "https://bigquery.googleapis.com/bigquery/v2".into(),
            service_account,
            token: Arc::new(Mutex::new(None)),
            timeout,
        })
    }

    /// Returns a current access token, refreshing it when the cached one has
    /// expired or is absent. The token is a secret: it is used only to set the
    /// Authorization header and never reaches an error message or log.
    pub(crate) async fn token(&self) -> Result<String, ConnectionError> {
        let mut guard = self.token.lock().await;
        if let Some(cached) = guard.as_ref()
            && cached.is_current()
        {
            return Ok(cached.token.clone());
        }
        let cached = exchange(&self.client, &self.service_account, self.timeout)
            .await
            .map_err(|_| errors::auth())?;
        let token = cached.token.clone();
        *guard = Some(cached);
        Ok(token)
    }

    /// Sends a POST carrying `body` and the bearer token. Callers decode the
    /// response status and body themselves.
    pub(crate) async fn post(
        &self,
        url: &str,
        token: &str,
        body: serde_json::Value,
    ) -> Result<reqwest::Response, ConnectionError> {
        let request = self.authorized(url, token, body);
        timeout(self.timeout, request.send())
            .await
            .map_err(|_| errors::query_timeout())?
            .map_err(errors::transport_query)
    }

    pub(crate) fn authorized(
        &self,
        url: &str,
        token: &str,
        body: serde_json::Value,
    ) -> RequestBuilder {
        self.client
            .post(url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"))
            .json(&body)
    }

    pub(crate) fn query_url(&self) -> String {
        format!("{}/projects/{}/queries", self.api_origin, self.project)
    }

    pub(crate) fn jobs_url(&self) -> String {
        format!("{}/projects/{}/jobs", self.api_origin, self.project)
    }
}

/// A GCP project id is letters, digits, and hyphens. Rejecting `/`, `?`, and
/// `:` keeps a configured project from escaping the path slot of the URL.
fn valid_project(project: &str) -> bool {
    !project.is_empty()
        && project.len() <= 255
        && project
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[async_trait]
impl DatabaseConnector for BigQueryConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::BigQuery
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        execute::ping(self).await
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        metadata::schema(self).await
    }

    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        execute::query(self, request).await
    }
}

#[cfg(test)]
mod tests {
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
}
