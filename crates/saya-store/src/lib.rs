//! Redacted session storage contracts for SAYA CLI.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod filesystem;

pub use filesystem::FsSessionStore;

/// Persistable session data. Callers must provide content after secret redaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactedSession {
    pub id: String,
    pub profile_names: Vec<String>,
    pub messages: Vec<RedactedMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactedMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("session store failed: {0}")]
    Failure(String),
}

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn save(&self, session: RedactedSession) -> Result<(), StoreError>;
    async fn load(&self, id: &str) -> Result<Option<RedactedSession>, StoreError>;
    async fn most_recent(&self) -> Result<Option<RedactedSession>, StoreError>;
}
