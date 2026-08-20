use thiserror::Error;

/// Errors exposed by database connectors without leaking driver details.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConnectionError {
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("authentication failed: {0}")]
    AuthenticationFailed(String),
    #[error("query failed: {0}")]
    QueryFailed(String),
    #[error("schema discovery failed: {0}")]
    SchemaFailed(String),
    #[error("invalid connection configuration: {0}")]
    InvalidConfiguration(String),
    #[error("query cancelled")]
    Cancelled,
    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

impl ConnectionError {
    pub fn connection_failed(message: impl Into<String>) -> Self {
        Self::ConnectionFailed(message.into())
    }

    pub fn authentication_failed(message: impl Into<String>) -> Self {
        Self::AuthenticationFailed(message.into())
    }

    pub fn query_failed(message: impl Into<String>) -> Self {
        Self::QueryFailed(message.into())
    }

    pub fn schema_failed(message: impl Into<String>) -> Self {
        Self::SchemaFailed(message.into())
    }

    pub fn invalid_configuration(message: impl Into<String>) -> Self {
        Self::InvalidConfiguration(message.into())
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported(message.into())
    }

    pub fn cancelled() -> Self {
        Self::Cancelled
    }
}
