//! Redacted session storage contracts for SAYA CLI.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

mod audit_store;
mod contracts;
mod error;
mod filesystem;
mod history;
mod knowledge_items;
mod migration;
mod redaction;
mod schema_store;
mod sqlite;
mod sqlite_support;
mod state_contracts;

pub use contracts::{ForgetReason, MAX_PREFERENCE_VALUE_BYTES, PreferenceStore};
pub use error::StoreError;
pub use filesystem::FsSessionStore;
pub use knowledge_items::{
    KnowledgeItem, KnowledgeItemRequest, KnowledgeItemStore, KnowledgeStoreError,
    MAX_KNOWLEDGE_ITEM_BYTES, knowledge_item_id_for,
};
pub use redaction::redact;
pub use sqlite::{OPEN_BUSY_CEILING, SqliteStateStore};
pub use sqlite_support::state_sidecar_path;
pub use state_contracts::{
    AuditEntry, AuditOperation, AuditRecord, AuditStatus, AuditStore, CachedSchema, SCHEMA_VERSION,
    SchemaCacheEntry, SchemaStore,
};

/// Persistable session data. Callers must provide content after secret redaction.
pub const SESSION_VERSION: u32 = 2;

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
    #[serde(default)]
    pub allow_data_sharing: bool,
    #[serde(default)]
    pub approval_mode: String,
    #[serde(default)]
    pub turns: Vec<RedactedTurn>,
    #[serde(default)]
    pub profile_names: Vec<String>,
    #[serde(default)]
    pub messages: Vec<RedactedMessage>,
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
            allow_data_sharing: false,
            approval_mode: String::new(),
            turns: Vec::new(),
            profile_names: Vec::new(),
            messages: Vec::new(),
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
    /// The tool-call arguments serialized to JSON exactly as the model sent
    /// them — for a SQL tool, `{"sql": "...", ...}`, so the statement the agent
    /// ran is persisted here. The SQL text is already user-visible in the
    /// transcript and on the `--format ndjson` event stream, so persisting it
    /// discloses nothing new; [`FsSessionStore::save`] redacts
    /// credential-shaped substrings in it the same way it redacts a user turn.
    /// Result rows are never stored — only [`Self::result_shape`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub arguments: String,
    /// The value-free shape of a query result: row count and column names.
    /// `None` for a tool that produced no row-shaped result (a denied call, a
    /// non-query tool, or a failed query). Carries no cell values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_shape: Option<RedactedToolResultShape>,
}

/// The value-free shape of a query result persisted per tool call: the row
/// count and the column names. Built by the agent's `result_shape_of` from a
/// result's `row_count` and `columns` keys alone — `rows` is never read — so a
/// planted cell value cannot reach a session file.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: String,
    pub modified_unix_ms: u128,
}

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn save(&self, session: RedactedSession) -> Result<(), StoreError>;
    async fn load(&self, id: &str) -> Result<Option<RedactedSession>, StoreError>;
    async fn most_recent(&self) -> Result<Option<RedactedSession>, StoreError>;
    async fn history(&self) -> Result<Vec<SessionSummary>, StoreError>;
}
