//! Provider and tool errors that surface from the agent protocol.

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderError {
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("provider returned an invalid response")]
    InvalidResponse,
    #[error("provider is not configured: {0}")]
    Configuration(String),
    #[error("provider stream was cancelled")]
    Cancelled,
}

impl ProviderError {
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ToolError {
    #[error("data sharing is disabled for this cloud provider")]
    DataSharingDisabled,
    #[error("invalid query arguments")]
    InvalidQueryArguments,
    #[error("unsupported read-only tool")]
    UnsupportedTool,
    #[error("invalid tool arguments: expected an object")]
    ArgumentsNotObject,
    #[error("invalid tool arguments: unsupported property")]
    UnsupportedProperty,
    #[error("invalid tool arguments: connection must be a string")]
    ConnectionNotString,
    #[error("invalid tool arguments: sql must be a string")]
    SqlNotString,
    #[error("no database profile is selected")]
    NoConnectionSelected,
    #[error("unknown connection \"{target}\"; available connections: {available}")]
    UnknownConnection { target: String, available: String },
    #[error("read-only query failed")]
    QueryFailed,
    #[error("read-only query failed: {0}")]
    QueryFailedDetail(String),
    #[error("read-only query timed out")]
    QueryTimedOut,
    #[error("query result unavailable")]
    QueryResultUnavailable,
    #[error("schema discovery failed: {0}")]
    SchemaDiscoveryFailed(String),
    #[error("{0}")]
    Chart(String),
}
