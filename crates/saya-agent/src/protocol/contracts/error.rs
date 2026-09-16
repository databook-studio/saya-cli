//! Provider and tool errors that surface from the agent protocol.

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProviderError {
    #[error("provider request failed: {0}")]
    Request(String),
    #[error("provider returned an invalid response")]
    InvalidResponse,
    /// The model hit its per-response output-token limit mid-answer. Distinct
    /// from a transport failure: re-sending the identical request fails
    /// identically, so the loop must not retry it. Carries the redacted
    /// partial output the wire had already emitted. Construct via
    /// [`ProviderError::output_truncated`], which redacts both partials —
    /// a struct literal would bypass that.
    #[error("provider response truncated: the model hit its output-token limit")]
    OutputTruncated {
        /// Text the wire emitted before the cap, redacted at construction.
        partial_text: String,
        /// Raw tool-argument fragments assembled before the cap, one per
        /// partial call, redacted at construction.
        partial_tool_json: Vec<String>,
    },
    #[error("provider is not configured: {0}")]
    Configuration(String),
    #[error("provider stream was cancelled")]
    Cancelled,
}

impl ProviderError {
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }

    /// Builds the truncation signal with both partials passed through the
    /// same redaction as every other model output, so a credential-shaped
    /// substring in a cut-off answer cannot leak through the error path.
    pub fn output_truncated(partial_text: String, partial_tool_json: Vec<String>) -> Self {
        Self::OutputTruncated {
            partial_text: saya_types::redact(&partial_text),
            partial_tool_json: partial_tool_json
                .iter()
                .map(|json| saya_types::redact(json))
                .collect(),
        }
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
    #[error("invalid tool arguments: content must be a string")]
    ContentNotString,
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
    /// A workspace read failed. The detail is the harness containment
    /// error's own text — path resolution, symlink refusal, bounds — so the
    /// model reads the real reason rather than a guess.
    #[error("workspace read failed: {0}")]
    Workspace(String),
    /// A workspace write failed. The detail is the harness containment
    /// error's own text, for the same reason [`ToolError::Workspace`] is.
    #[error("workspace write failed: {0}")]
    WorkspaceWrite(String),
    /// The content argument of a workspace write exceeded the tool's byte
    /// bound. Typed — and refused whole, never truncated: a partially
    /// written file is worse than a refused one.
    #[error("workspace write refused: content is over the {limit}-byte write bound")]
    WorkspaceWriteTooLarge { limit: usize },
    /// A runner tool refused or failed. The detail is the harness runner
    /// error's own text — the typed refusal (allowlist, argv shape, sandbox,
    /// timeout) or failure, so the model reads the real reason, never a
    /// generic failure.
    #[error("{0}")]
    Runner(String),
    /// A fetch tool refused or failed. The detail is the harness fetch
    /// error's own text — the typed egress refusal, byte/wall-clock bound,
    /// status, or budget trip — so the model reads the real reason, never a
    /// generic failure.
    #[error("{0}")]
    Fetch(String),
}
