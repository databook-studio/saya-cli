use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use tokio::sync::Mutex;

use crate::{ConnectorOptions, DatabaseConnector};

use super::{auth::Auth, cancellation, errors, legacy, metadata, protocol_v2};

pub struct SnowflakeConnector {
    pub(crate) client: reqwest::Client,
    pub(crate) origin: String,
    pub(crate) account: String,
    pub(crate) user: String,
    pub(crate) auth: Auth,
    pub(crate) context: Context,
    pub(crate) timeout: Duration,
    pub(crate) active: Arc<Mutex<Option<String>>>,
}

#[derive(Clone)]
pub(crate) struct Context {
    pub(crate) warehouse: Option<String>,
    pub(crate) database: Option<String>,
    pub(crate) schema: Option<String>,
    pub(crate) role: Option<String>,
}

impl SnowflakeConnector {
    pub(crate) fn new(
        account: String,
        user: String,
        auth: Auth,
        context: Context,
        options: ConnectorOptions,
    ) -> Result<Self, ConnectionError> {
        let host_account = account.trim().to_ascii_lowercase();
        if !valid_account(&host_account) {
            return Err(ConnectionError::InvalidConfiguration(
                "invalid Snowflake account identifier".into(),
            ));
        }
        let timeout = Duration::from_secs(options.query_timeout_seconds.max(1));
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let client = reqwest::Client::builder()
            .user_agent("saya-cli/0.1")
            .default_headers(headers)
            .connect_timeout(timeout)
            .timeout(timeout)
            .build()
            .map_err(|_| errors::connect())?;
        Ok(Self {
            client,
            origin: format!("https://{host_account}.snowflakecomputing.com"),
            account: host_account.split('.').next().unwrap_or_default().into(),
            user,
            auth,
            context,
            timeout,
            active: Arc::new(Mutex::new(None)),
        })
    }
}

fn valid_account(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|item| item.is_ascii_alphanumeric() || item == b'-')
        })
}

#[async_trait]
impl DatabaseConnector for SnowflakeConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Snowflake
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        match &self.auth {
            Auth::ExternalBrowser { enabled: false } => Err(ConnectionError::Unsupported(
                "Snowflake external-browser authentication requires interactive mode".into(),
            )),
            Auth::ExternalBrowser { enabled: true } => Err(ConnectionError::Unsupported("Snowflake external-browser session exchange is deferred until CLI interactive wiring".into())),
            Auth::Keypair(_) => self.execute(QueryRequest::new("SELECT 1", 1)).await.map(|_| ()),
            Auth::Userpass(_) => legacy::login(self).await.map(|_| ()),
        }
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        metadata::schema(self).await
    }
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        match &self.auth {
            Auth::Keypair(_) => protocol_v2::execute(self, request).await,
            Auth::Userpass(_) => legacy::execute(self, request).await,
            Auth::ExternalBrowser { .. } => Err(ConnectionError::Unsupported(
                "Snowflake external-browser authentication requires interactive mode".into(),
            )),
        }
    }
    async fn cancel(&self) -> Result<(), ConnectionError> {
        cancellation::cancel(self).await
    }
}
