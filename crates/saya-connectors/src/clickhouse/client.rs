use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, RequestBuilder, Response};
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use tokio::time::timeout;

use crate::{ConnectorOptions, DatabaseConnector};

use super::errors;

/// Connects to ClickHouse over its HTTP interface. Every statement a caller
/// supplies is first narrowed to a read by the shared safety layer; this client
/// only carries the result back over the wire.
pub struct ClickHouseConnector {
    pub(crate) client: Client,
    pub(crate) endpoint: String,
    pub(crate) database: Option<String>,
    pub(crate) user: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) query_timeout: Duration,
    /// Server-side wall-clock bound passed as the `max_execution_time` setting,
    /// in seconds. The client also wraps each request in [`query_timeout`], so
    /// the server setting is a second bound, not a replacement for the local one.
    pub(crate) max_execution_time: u64,
}

impl ClickHouseConnector {
    pub(crate) fn new(
        host: String,
        port: Option<u16>,
        database: Option<String>,
        user: Option<String>,
        password: Option<String>,
        secure: Option<bool>,
        settings: ConnectorOptions,
    ) -> Result<Self, ConnectionError> {
        if !valid_host(&host) {
            return Err(ConnectionError::invalid_configuration(
                "invalid ClickHouse host",
            ));
        }
        let https = secure.unwrap_or(false);
        // Basic auth puts the password in a header, so plain HTTP to anywhere
        // but this machine hands it to the network. ClickHouse's own default is
        // port 8123 without TLS, which is fine for a local server and wrong for
        // a remote one, so a remote profile has to say `secure = true` rather
        // than have the downgrade happen silently.
        if !https && password.is_some() && !is_loopback(&host) {
            return Err(ConnectionError::invalid_configuration(
                "ClickHouse password over plain HTTP to a remote host: set secure = true",
            ));
        }
        let query_timeout = Duration::from_secs(settings.query_timeout_seconds.max(1));
        let max_execution_time = settings.query_timeout_seconds.max(1);
        let port = port.unwrap_or(if https { 8443 } else { 8123 });
        let scheme = if https { "https" } else { "http" };
        let endpoint = format!("{scheme}://{host}:{port}/");
        let client = Client::builder()
            .connect_timeout(query_timeout)
            .timeout(query_timeout)
            .build()
            .map_err(|_| errors::build())?;
        Ok(Self {
            client,
            endpoint,
            database,
            user,
            password,
            query_timeout,
            max_execution_time,
        })
    }

    /// Builds the POST that sends `sql` with the connector's wire format and
    /// bounds. The body is `<sql> FORMAT JSON` because the connector parses the
    /// `meta`/`data` shape; the safety layer has already rejected any
    /// caller-supplied `FORMAT`, so the two never collide. Server-side bounds
    /// (`max_result_rows`, `max_execution_time`) run in addition to the
    /// client-side row cap and timeout.
    pub(crate) fn build_request(&self, sql: &str, max_rows: usize) -> RequestBuilder {
        let body = wire_body(sql);
        let max_result_rows = max_rows.saturating_add(1) as u64;
        let mut request = self
            .client
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "text/plain")
            .body(body);
        if let Some(database) = &self.database {
            request = request.query(&[("database", database.as_str())]);
        }
        request = request
            .query(&[("max_result_rows", &max_result_rows)])
            .query(&[("max_execution_time", &self.max_execution_time)]);
        if let Some(user) = &self.user {
            request = request.basic_auth(user, self.password.as_deref());
        }
        request
    }

    /// Sends a query-context request and maps transport failures to query
    /// errors. Callers check the response status and decode the body.
    pub(crate) async fn post(
        &self,
        sql: &str,
        max_rows: usize,
    ) -> Result<Response, ConnectionError> {
        timeout(self.query_timeout, self.build_request(sql, max_rows).send())
            .await
            .map_err(|_| errors::query_timeout())?
            .map_err(errors::transport_query)
    }
}

/// The wire body sent for a query: the caller's SQL followed by the connector's
/// output format. Kept separate so the format the connector parses is decided
/// in one place and never mixed into the request builder.
fn wire_body(sql: &str) -> String {
    format!("{sql} FORMAT JSON")
}

/// A host is a bare hostname or IPv4 address: letters, digits, dots, hyphens,
/// and underscores. Rejecting whitespace and `/` keeps a configured host from
/// escaping the host slot of the URL into a path or scheme.
/// Whether the host names this machine, where plain HTTP keeps the password
/// off any network.
fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
}

#[async_trait]
impl DatabaseConnector for ClickHouseConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::ClickHouse
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        let response = timeout(self.query_timeout, self.build_request("SELECT 1", 1).send())
            .await
            .map_err(|_| ConnectionError::connection_failed("ClickHouse connection timed out"))?
            .map_err(errors::transport_connect)?;
        if !response.status().is_success() {
            return Err(errors::connect_status(response.status()));
        }
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        super::metadata::schema(self).await
    }

    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        super::execute::query(self, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_body_appends_format_json() {
        // The connector owns the wire format; the safety layer has already
        // rejected any caller-supplied FORMAT, so the body is always the SQL
        // followed by exactly one FORMAT clause.
        assert_eq!(
            wire_body("SELECT * FROM t LIMIT 11"),
            "SELECT * FROM t LIMIT 11 FORMAT JSON"
        );
        assert_eq!(wire_body("SELECT 1"), "SELECT 1 FORMAT JSON");
    }

    #[test]
    fn construction_picks_http_and_default_port() {
        let connector = ClickHouseConnector::new(
            // Loopback, because plain HTTP with a password is refused for a
            // remote host and this is checking scheme and port, not auth.
            "127.0.0.1".into(),
            None,
            Some("analytics".into()),
            Some("reader".into()),
            Some("shh".into()),
            None,
            ConnectorOptions::default(),
        )
        .unwrap();
        // The endpoint is the root path over plain HTTP when secure is unset.
        assert_eq!(connector.endpoint, "http://127.0.0.1:8123/");
        assert_eq!(connector.max_execution_time, 60);
    }

    #[test]
    fn https_selects_8443_and_https_scheme() {
        let connector = ClickHouseConnector::new(
            "db.example.test".into(),
            None,
            None,
            None,
            None,
            Some(true),
            ConnectorOptions::default(),
        )
        .unwrap();
        assert_eq!(connector.endpoint, "https://db.example.test:8443/");
    }

    #[test]
    fn invalid_host_is_rejected_before_any_request() {
        // The connector deliberately does not implement Debug because it holds
        // the resolved password, so the error is read by matching rather than
        // by `unwrap_err` (which would require a Debug impl that could leak it).
        let result = ClickHouseConnector::new(
            "db.example.test/path".into(),
            None,
            None,
            None,
            None,
            None,
            ConnectorOptions::default(),
        );
        let error = match result {
            Ok(_) => panic!("an invalid host must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(error, ConnectionError::InvalidConfiguration(_)));
    }
}
