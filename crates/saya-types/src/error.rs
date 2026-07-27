use thiserror::Error;

/// Errors exposed by database connectors without leaking driver details.
#[derive(Debug, Error)]
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
