use crate::StoreError;
use async_trait::async_trait;
use saya_types::SchemaTree;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct CachedSchema {
    pub schema: SchemaTree,
    pub updated_unix_ms: i64,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaCacheEntry {
    pub profile_id: String,
    pub updated_unix_ms: i64,
    pub version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOperation {
    ConnectionTest,
    SchemaRefresh,
    Query,
    AgentQuery,
}
impl AuditOperation {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ConnectionTest => "connection_test",
            Self::SchemaRefresh => "schema_refresh",
            Self::Query => "query",
            Self::AgentQuery => "agent_query",
        }
    }
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "connection_test" => Some(Self::ConnectionTest),
            "schema_refresh" => Some(Self::SchemaRefresh),
            "query" => Some(Self::Query),
            "agent_query" => Some(Self::AgentQuery),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditStatus {
    Success,
    Failure,
    Cached,
}
impl AuditStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Cached => "cached",
        }
    }
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "success" => Some(Self::Success),
            "failure" => Some(Self::Failure),
            "cached" => Some(Self::Cached),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub profile_id: String,
    pub operation: AuditOperation,
    pub status: AuditStatus,
    pub duration_ms: u64,
    pub row_count: Option<usize>,
    pub truncated: Option<bool>,
    pub session_id: Option<String>,
}
impl AuditEntry {
    pub fn new(
        profile_id: impl Into<String>,
        operation: AuditOperation,
        status: AuditStatus,
        duration_ms: u64,
    ) -> Self {
        Self {
            profile_id: profile_id.into(),
            operation,
            status,
            duration_ms,
            row_count: None,
            truncated: None,
            session_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    pub created_unix_ms: i64,
    pub event: AuditEntry,
}

#[async_trait]
pub trait SchemaStore: Send + Sync {
    async fn upsert_schema(&self, profile_id: &str, schema: &SchemaTree) -> Result<(), StoreError>;
    async fn get_schema(&self, profile_id: &str) -> Result<Option<CachedSchema>, StoreError>;
    async fn invalidate_schema(&self, profile_id: &str) -> Result<(), StoreError>;
    async fn list_schema_metadata(&self) -> Result<Vec<SchemaCacheEntry>, StoreError>;
}

#[async_trait]
pub trait AuditStore: Send + Sync {
    async fn record_audit(&self, entry: AuditEntry) -> Result<(), StoreError>;
    async fn recent_audit(&self, limit: usize) -> Result<Vec<AuditRecord>, StoreError>;
}
