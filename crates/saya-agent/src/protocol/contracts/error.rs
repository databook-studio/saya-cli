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
    #[error("invalid tool arguments: path must be a string")]
    PathNotString,
    #[error("invalid tool arguments: pattern must be a string")]
    PatternNotString,
    #[error("invalid tool arguments: case_insensitive must be a boolean")]
    CaseInsensitiveNotBool,
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
    #[error("no workspace is available in this run")]
    WorkspaceUnavailable,
    /// A workspace operation failed. The detail is the harness containment
    /// error's own text — path resolution, symlink refusal, bounds — so the
    /// model reads the real reason rather than a guess.
    #[error("workspace read failed: {0}")]
    Workspace(String),
}
