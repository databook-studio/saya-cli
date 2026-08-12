//! Redacted session storage contracts for SAYA CLI.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod audit_store;
mod filesystem;
mod history;
mod migration;
mod redaction;
mod schema_store;
mod sqlite;
mod sqlite_support;
mod state_contracts;

pub use filesystem::FsSessionStore;
pub use redaction::redact;
pub use sqlite::SqliteStateStore;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedactedToolMetadata {
    pub name: String,
    pub status: String,
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

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreError {
    #[error("local state store is unavailable")]
    Unavailable,
    #[error("the requested record does not exist")]
    NotFound,
    #[error("the record conflicts with an existing record")]
    Conflict,
    #[error("the value exceeds a store limit")]
    LimitExceeded,
    #[error("the value is not valid for storage")]
    Invalid,
    #[error("the state database was written by a newer version of saya")]
    VersionUnsupported,
}

impl StoreError {
    pub fn unavailable() -> Self {
        Self::Unavailable
    }
    pub fn not_found() -> Self {
        Self::NotFound
    }
    pub fn conflict() -> Self {
        Self::Conflict
    }
    pub fn limit_exceeded() -> Self {
        Self::LimitExceeded
    }
    pub fn invalid() -> Self {
        Self::Invalid
    }
}

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn save(&self, session: RedactedSession) -> Result<(), StoreError>;
    async fn load(&self, id: &str) -> Result<Option<RedactedSession>, StoreError>;
    async fn most_recent(&self) -> Result<Option<RedactedSession>, StoreError>;
    async fn history(&self) -> Result<Vec<SessionSummary>, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::StoreError;

    #[test]
    fn errors_are_payload_free_and_fieldless() {
        let errors = [
            StoreError::Unavailable,
            StoreError::NotFound,
            StoreError::Conflict,
            StoreError::LimitExceeded,
            StoreError::Invalid,
            StoreError::VersionUnsupported,
        ];
        for error in errors {
            let rendered = error.to_string();
            assert!(!rendered.is_empty());
            assert!(!rendered.contains('{'));
            assert!(!rendered.contains(':'));
            assert!(!rendered.contains("SUPERSECRETVALUE"));
        }
    }
}
