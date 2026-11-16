//! Tool-call failures that surface from the agent protocol.

/// A refused or failed tool call, carrying the typed reason the model reads.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
    /// A `grep` `type` named a language outside the bounded extension map.
    /// Typed, and carrying the names that do exist, so the model corrects the
    /// argument in one turn instead of guessing. The requested name is
    /// truncated by the caller; nothing was searched.
    #[error(
        "invalid tool arguments: unsupported grep type \"{requested}\"; supported: {supported}"
    )]
    GrepTypeUnsupported {
        requested: String,
        supported: &'static str,
    },
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
    #[error("no workspace is available in this run")]
    WorkspaceUnavailable,
    /// Actual contained filesystem absence, distinct from opaque read failures.
    #[error("workspace read failed: file does not exist")]
    WorkspaceFileMissing,
    /// A workspace read failed. The detail is the harness containment
    /// error's own text — path resolution, symlink refusal, bounds — so the
    /// model reads the real reason rather than a guess.
    #[error("workspace read failed: {0}")]
    Workspace(String),
    /// A workspace read of an image whose bytes exceed the image bound.
    /// Refused whole — an image is never truncated to fit, because half an
    /// image is not the file — naming the bound and the file's size so the
    /// model stops retrying the same read. Nothing was read out.
    #[error(
        "workspace read refused: image is over the {limit}-byte image bound (found {found}): {path}"
    )]
    WorkspaceImageTooLarge {
        path: String,
        limit: u64,
        found: u64,
    },
    /// A workspace read asked for a line range of a file it recognised as an
    /// image. An image has no lines, so the range cannot select anything: the
    /// call is refused rather than answered with a lossy text view of the
    /// image's bytes. Nothing was read out.
    #[error("workspace read refused: start_line/line_count do not apply to an image: {path}")]
    WorkspaceImageLineRange { path: String },
    /// A workspace write failed. The detail is the harness containment
    /// error's own text, for the same reason [`ToolError::Workspace`] is.
    #[error("workspace write failed: {0}")]
    WorkspaceWrite(String),
    /// The content argument of a workspace write exceeded the tool's byte
    /// bound. Typed — and refused whole, never truncated: a partially
    /// written file is worse than a refused one.
    #[error("workspace write refused: content is over the {limit}-byte write bound")]
    WorkspaceWriteTooLarge { limit: usize },
    /// A workspace write or edit named an existing file and supplied no
    /// digest — and no read of that file earlier in this run supplied one
    /// either. Carries the file's current size and digest so the model
    /// retries with the state it was just handed rather than spending a read
    /// turn first. Nothing was written.
    #[error(
        "workspace {operation} refused: this file exists and was not changed; pass expected_digest \
         from a fresh read (or the previous write/edit result) \
         (size: {size}, current_digest: {current_digest}): {path}"
    )]
    WorkspaceDigestRequired {
        /// The refusing operation: `write`, `edit`, or `append`.
        operation: &'static str,
        path: String,
        size: u64,
        current_digest: String,
    },
    /// A workspace write's `expected_digest` no longer matches the file's
    /// current bytes: the file moved, and no write landed. Carries the current
    /// size and digest so the model retries without a separate read.
    #[error(
        "workspace write refused: the file changed since it was observed (expected digest: {expected_digest}, current size: {current_size}, current_digest: {current_digest}): {path}"
    )]
    WorkspaceWriteMoved {
        path: String,
        expected_digest: String,
        current_size: u64,
        current_digest: String,
    },
    /// A workspace edit found no anchor match: the count, the file's current
    /// size and digest, and `hint` — bounded candidate regions, empty if none.
    #[error(
        "workspace edit refused: anchor matched {matches} time(s), need exactly 1 (size: {size}, digest: {digest}): {path}{hint}"
    )]
    WorkspaceEditNoMatch {
        path: String,
        matches: usize,
        size: u64,
        digest: String,
        hint: String,
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
    /// write. Binary support is out of scope here.
    #[error("workspace edit refused: file is not UTF-8 text: {path}")]
    WorkspaceEditNotText { path: String },
    /// A workspace edit failed inside the harness containment layer. The
    /// detail is the harness error's own text, so the model reads the real
    /// reason rather than a guess. Carries counts and sizes, never content.
    #[error("workspace edit failed: {0}")]
    WorkspaceEdit(String),
    /// A `notebook_edit` call named a `mode` outside the three the tool
    /// serves. Typed, and carrying the names that do exist, so the model
    /// corrects the argument in one turn. Nothing was read or written.
    #[error(
        "invalid tool arguments: unsupported notebook edit mode \"{requested}\"; supported: replace, insert, delete"
    )]
    NotebookModeUnsupported { requested: String },
    /// A `notebook_edit` call refused the notebook it was pointed at: the
    /// file is not a notebook, the cell it named is not there, or the cell id
    /// it named is not unique. The detail is the one true reason, never file
    /// content. Nothing was written.
    #[error("notebook edit refused: {detail}: {path}")]
    WorkspaceNotebook { path: String, detail: String },
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
    /// A workspace edit's `replace_all` argument was not a boolean.
    #[error("invalid tool arguments: replace_all must be a boolean")]
    ReplaceAllNotBool,
    /// A workspace edit combined `replace_all` with another edit shape —
    /// `edits`, or `offset`+`chunk`. Refused rather than guessing which shape
    /// was meant. No write.
    #[error("invalid tool arguments: replace_all applies only to one old_text/new_text anchor")]
    ReplaceAllConflict,
    /// A workspace edit combined its shapes — `old_text`/`new_text` with
    /// `offset`/`chunk`, or `edits` with either. Refused rather than guessing
    /// which shape was meant. No write.
    #[error(
        "invalid tool arguments: use one edit shape — `old_text`/`new_text`, \
         `offset`/`chunk`, or `edits`"
    )]
    EditShapeConflict,
    /// A shell call asked for a background run and a timeout: the deadline
    /// would be ignored, so it is refused rather than silently dropped.
    #[error(
        "invalid tool arguments: `timeout` does not apply to a background run; \
         omit `timeout` or `run_in_background`"
    )]
    ShellTimeoutWithBackground,
    /// A `shell` command over the byte ceiling; nothing ran.
    #[error("invalid tool arguments: the command is {bytes} bytes; the ceiling is {ceiling}")]
    ShellCommandTooLong { bytes: usize, ceiling: usize },
    /// A runner tool refused or failed. The detail is the harness runner
    /// error's own text — the typed refusal or failure — so the model reads
    /// the real reason, never a generic failure.
    #[error("{0}")]
    Runner(String),
    /// A fetch tool refused or failed. The detail is the harness fetch
    /// error's own text — the typed egress refusal, byte/wall-clock bound,
    /// status, or budget trip — so the model reads the real reason, never a
    /// generic failure.
    #[error("{0}")]
    Fetch(String),
    /// A `checklist` call whose `tasks` argument was not an array, or whose
    /// array did not deserialize as a task list. Payload-free: the model
    /// fixes the shape, never reads back what it sent.
    #[error("invalid tool arguments: tasks must be an array of tasks")]
    TasksNotArray,
    /// A `checklist` list holding more than the contract's task bound.
    /// Payload-free — the bound, not the rejected list.
    #[error("task list refused: task list holds at most 32 items")]
    TooManyTasks,
    /// A `checklist` list with more than one task in progress.
    #[error("task list refused: only one task may be in progress")]
    TooManyInProgress,
    /// A `checklist` task whose title is empty.
    #[error("task list refused: task title must not be empty")]
    TaskTitleEmpty,
    /// A `checklist` task whose title exceeds the contract's title bound.
    #[error("task list refused: task title is too long")]
    TaskTitleTooLong,
    /// A `checklist` task whose title carries control characters.
    #[error("task list refused: task title contains control characters")]
    TaskTitleControl,
    /// A `checklist` task whose note exceeds the contract's note bound.
    #[error("task list refused: task note is too long")]
    TaskNoteTooLong,
    /// A `checklist` task whose note carries control characters.
    #[error("task list refused: task note contains control characters")]
    TaskNoteControl,
    /// A `PreToolUse` hook refused this call. The detail is the hook's own
    /// stderr — bounded and already redacted by the runner's capture — so the
    /// model reads the reason the user's script gave rather than a generic
    /// refusal. Nothing ran. The executor wraps this in
    /// [`ToolError::NoEffect`], because a call refused before it started
    /// provably changed nothing.
    #[error("blocked by a PreToolUse hook: {0}")]
    HookBlocked(String),
    /// The executor that raised this error proved the tool changed nothing:
    /// the refusal was raised before any write or other effect. Only an
    /// executor that knows its tool may wrap; the wrapper forwards the inner
    /// error's text and source so the model reads the exact same message.
    #[error(transparent)]
    NoEffect(Box<ToolError>),
}

impl ToolError {
    /// Whether the executor that raised this error proved the tool changed
    /// nothing. Only [`ToolError::NoEffect`] carries that proof: an inner
    /// error on its own never does, so the settlement stays conservative.
    pub fn proves_no_effect(&self) -> bool {
        matches!(self, ToolError::NoEffect(_))
    }
}
