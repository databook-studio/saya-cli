//! The persisted session record: redacted, versioned, serde-stable.

use serde::{Deserialize, Serialize};

use crate::continuation::StoredContinuation;

/// Persistable session data. Callers must provide content after secret redaction.
pub const SESSION_VERSION: u32 = 2;

/// A successor session's lineage link to the session it was rotated from.
/// LINEAGE ONLY: the link records where a session came from and nothing
/// else — a successor inherits no grants, approvals, permissions, or
/// capabilities of any kind. Additive at `SESSION_VERSION` 2 with
/// `#[serde(default)]`: records written before rotation existed carry no
/// link and re-serialize without the key. An older binary reading a record
/// that carries a link silently drops the unknown field, and downgrade
/// writes are unsupported without restoring a pre-upgrade backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentLink {
    /// The parent session's id.
    pub id: String,
    /// The rotation that produced this successor; repeating the same
    /// `(parent, rotation_id)` request is idempotent.
    pub rotation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactedSession {
    #[serde(default = "legacy_session_version")]
    pub version: u32,
    pub id: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub included_profiles: Vec<String>,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    /// The provider endpoint selected for the session, when known. Kept as a
    /// URL string only; no credentials belong in a session record.
    #[serde(default)]
    pub provider_endpoint: Option<String>,
    /// Whether the endpoint field was explicitly bound, including an
    /// explicit clear after a provider switch.
    #[serde(default)]
    pub provider_endpoint_bound: bool,
    #[serde(default)]
    pub allow_data_sharing: bool,
    #[serde(default)]
    pub approval_mode: String,
    /// The session's task posture (`"build"` or `"plan"`), beside
    /// `approval_mode`. An empty or missing value resumes as build — the
    /// behaviour of every session written before this field existed. New
    /// values stay additive `#[serde(default)]` fields, never a version
    /// bump: the restore path treats absence as the default, and version 2
    /// already marks the shape the `version < SESSION_VERSION` guards read.
    #[serde(default)]
    pub agent_mode: String,
    /// The session's pinned workspace root, canonical, resolved once at
    /// first start. Absent (`None`) on every session written before the
    /// workspace existed — such a session resumes unbound, its old
    /// behaviour.
    #[serde(default)]
    pub workspace_root: Option<String>,
    /// The session's task list: conversation metadata tracking what a
    /// building session is working on, binding no authority. An additive
    /// `#[serde(default)]` field at `SESSION_VERSION` 2 — the `agent_mode`
    /// precedent — so a record written before it existed resumes with an
    /// empty list and no version bump is owed.
    #[serde(default)]
    pub task_list: saya_types::SessionTaskList,
    #[serde(default)]
    pub turns: Vec<RedactedTurn>,
    #[serde(default)]
    pub profile_names: Vec<String>,
    #[serde(default)]
    pub messages: Vec<RedactedMessage>,
    /// The session's optional continuation, additive at `SESSION_VERSION` 2
    /// beside `task_list` — legacy records carry none and re-serialize
    /// without the key. A typed record is redacted and revalidated at the
    /// save boundary; a continuation this binary does not understand loads
    /// as [`StoredContinuation::Unsupported`], is preserved verbatim, and
    /// cannot be overwritten by a save that would drop it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<StoredContinuation>,
    /// The session's optional rotation lineage link to its parent session,
    /// additive at `SESSION_VERSION` 2 beside `continuation` — legacy
    /// records carry none and re-serialize without the key. The link is
    /// lineage only: a successor inherits no grants, approvals, or
    /// permissions. Rotation sets and validates it in the store; a plain
    /// save preserves whatever a loaded record carried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ParentLink>,
}

impl Default for RedactedSession {
    fn default() -> Self {
        Self {
            version: SESSION_VERSION,
            id: String::new(),
            profile: None,
            included_profiles: Vec::new(),
            provider: String::new(),
            model: String::new(),
            provider_endpoint: None,
            provider_endpoint_bound: false,
            allow_data_sharing: false,
            approval_mode: String::new(),
            agent_mode: String::new(),
            workspace_root: None,
            task_list: saya_types::SessionTaskList::default(),
            turns: Vec::new(),
            profile_names: Vec::new(),
            messages: Vec::new(),
            continuation: None,
            parent: None,
        }
    }
}

fn legacy_session_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedactedTurn {
    pub user: String,
    pub assistant: String,
    #[serde(default)]
    pub database_derived: bool,
    #[serde(default)]
    pub tools: Vec<RedactedToolMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RedactedToolMetadata {
    pub name: String,
    pub status: String,
    /// Tool-call arguments are retained only while the live session needs
    /// them (for replay and compaction). They are intentionally absent from
    /// the persisted session format: arguments can contain SQL, paths, and
    /// other user data beyond the minimal name/status audit metadata.
    #[serde(skip)]
    pub arguments: String,
    /// The value-free result shape is likewise live-session metadata only.
    /// Persisting it is unnecessary for resuming provider history and would
    /// retain database schema details in a session record.
    #[serde(skip)]
    pub result_shape: Option<RedactedToolResultShape>,
}

/// The value-free shape of a query result kept for the live replay surface.
/// It is not part of the persisted session format. Built by the agent's
/// `result_shape_of` from a result's `row_count` and `columns` keys alone —
/// `rows` is never read — so a planted cell value cannot reach session state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RedactedToolResultShape {
    pub row_count: u64,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactedMessage {
    pub role: String,
    pub content: String,
}
