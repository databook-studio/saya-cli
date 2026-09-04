use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use tokio::sync::Mutex;

use crate::{ConnectorOptions, DatabaseConnector};

use super::{auth::Auth, browser, cancellation, errors, legacy, metadata, protocol_v2, sso};

pub struct SnowflakeConnector {
    pub(crate) client: reqwest::Client,
    pub(crate) origin: String,
    pub(crate) account: String,
    pub(crate) user: String,
    pub(crate) auth: Auth,
    pub(crate) context: Context,
    pub(crate) timeout: Duration,
    /// Serializes executes so the single active-query ID used for cancellation is unambiguous.
    pub(crate) in_flight: Arc<Mutex<()>>,
    pub(crate) active: Arc<Mutex<Option<String>>>,
    pub(crate) browser_opener: fn(&str) -> Result<(), ()>,
    pub(crate) sso_timeout: Duration,
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
            return Err(ConnectionError::invalid_configuration(
                "invalid Snowflake account identifier",
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
            in_flight: Arc::new(Mutex::new(())),
            active: Arc::new(Mutex::new(None)),
            browser_opener: browser::open,
            sso_timeout: sso::auth_timeout(),
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
            Auth::ExternalBrowser(_) => legacy::login(self).await.map(|_| ()),
            Auth::Keypair(_) => self
                .execute(QueryRequest::new("SELECT 1", 1))
                .await
                .map(|_| ()),
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
            Auth::ExternalBrowser(_) => legacy::execute(self, request).await,
        }
    }
    async fn cancel(&self) -> Result<(), ConnectionError> {
        cancellation::cancel(self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snowflake::auth;

    #[tokio::test]
    async fn test_in_flight_mutex_serializes() {
        let connector = SnowflakeConnector::new(
            "account".into(),
            "user".into(),
            Auth::Userpass(auth::Userpass {
                password: "pass".into(),
                token: Arc::new(Mutex::new(None)),
            }),
            Context {
                warehouse: None,
                database: None,
                schema: None,
                role: None,
            },
            ConnectorOptions::default(),
        )
        .unwrap();

        let guard = connector.in_flight.try_lock();
        assert!(
            guard.is_ok(),
            "in_flight mutex should be initially unlocked"
        );

        let second_guard = connector.in_flight.try_lock();
        assert!(
            second_guard.is_err(),
            "in_flight mutex should be locked while guard held"
        );

        drop(guard);
        let third_guard = connector.in_flight.try_lock();
        assert!(
            third_guard.is_ok(),
            "in_flight mutex should be lockable after drop"
        );
    }
}
