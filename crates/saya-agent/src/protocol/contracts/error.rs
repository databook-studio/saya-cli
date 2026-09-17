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
    /// A workspace edit found no anchor match. Typed, naming the count plus
    /// the file's current size and digest so the model can re-anchor — and
    /// writing nothing. Never carries file content.
    #[error(
        "workspace edit refused: anchor matched {matches} time(s), need exactly 1 (size: {size}, digest: {digest}): {path}"
    )]
    WorkspaceEditNoMatch {
        path: String,
        matches: usize,
        size: u64,
        digest: String,
    },
    /// A workspace edit found the anchor more than once. Typed, naming the
    /// count plus bounded line numbers — never "first wins", never file
    /// content. The model retries with a longer `old_text`.
    #[error(
        "workspace edit refused: anchor is ambiguous (matches: {matches}, lines: {lines:?}, size: {size}, digest: {digest}): {path}"
    )]
    WorkspaceEditAmbiguous {
        path: String,
        matches: usize,
        lines: Vec<u64>,
        size: u64,
        digest: String,
    },
    /// A workspace edit's `expected_size`/`expected_digest` precondition no
    /// longer matches the file's current state: the anchor moved under the
    /// model. No write. The model re-reads and retries.
    #[error(
        "workspace edit refused: file changed since measured (expected size: {expected_size:?}, current size: {current_size}, expected digest: {expected_digest:?}, current digest: {current_digest}): {path}"
    )]
    WorkspaceEditMoved {
        path: String,
        expected_size: Option<u64>,
        current_size: u64,
        expected_digest: Option<String>,
        current_digest: String,
    },
    /// An empty `old_text` matches everywhere, so it is a validation error —
    /// never an edit. No write.
    #[error("workspace edit refused: anchor must not be empty: {path}")]
    WorkspaceEditEmptyAnchor { path: String },
    /// The anchor or the replacement of a workspace edit exceeded the
    /// tool's byte bound. Typed — refused whole, never truncated.
    #[error(
        "workspace edit refused: argument is over the {limit}-byte bound (found {found}): {path}"
    )]
    WorkspaceEditTooLarge {
        path: String,
        limit: usize,
        found: usize,
    },
    /// A workspace edit refused a non-UTF-8 target: anchors are byte-exact
    /// strings and reads are lossy, so the anchor cannot be trusted. No
    /// write. Binary support is a later slice, not this one.
    #[error("workspace edit refused: file is not UTF-8 text: {path}")]
    WorkspaceEditNotText { path: String },
    /// A workspace edit failed inside the harness containment layer. The
    /// detail is the harness error's own text, so the model reads the real
    /// reason rather than a guess. Carries counts and sizes, never content.
    #[error("workspace edit failed: {0}")]
    WorkspaceEdit(String),
    /// A workspace edit's `old_text` argument was not a string.
    #[error("invalid tool arguments: old_text must be a string")]
    OldTextNotString,
    /// A workspace edit's `new_text` argument was not a string.
    #[error("invalid tool arguments: new_text must be a string")]
    NewTextNotString,
    /// A workspace append's `chunk` argument was not a string. Distinct from
    /// [`ToolError::NewTextNotString`] so the model fixes the right argument
    /// of the right variant.
    #[error("invalid tool arguments: chunk must be a string")]
    ChunkNotString,
    /// A workspace append's `offset` argument was not a non-negative integer.
    #[error("invalid tool arguments: offset must be a non-negative integer")]
    OffsetNotUint,
    /// A workspace append's `offset` no longer matches the file's current
    /// size: the model measured stale state. No write. The refusal names the
    /// current size and digest so the model resumes from `offset` — never a
    /// guess, never a partial chunk. Never carries file content.
    #[error(
        "workspace append refused: offset moved (expected offset: {expected_offset}, current size: {current_size}, current digest: {current_digest}): {path}"
    )]
    WorkspaceAppendOffset {
        path: String,
        expected_offset: u64,
        current_size: u64,
        current_digest: String,
    },
    /// A workspace edit's `expected_size` argument was not a non-negative
    /// integer.
    #[error("invalid tool arguments: expected_size must be a non-negative integer")]
    ExpectedSizeNotUint,
    /// A workspace edit's `expected_digest` argument was not a string.
    #[error("invalid tool arguments: expected_digest must be a string")]
    ExpectedDigestNotString,
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
