use super::*;
use async_trait::async_trait;
use saya_agent::{ChatMessage, ChatRequest};
use saya_agent::{ChatProvider, ChatResponse, ReasoningEffort, ResponseFormat};
use saya_connectors::DatabaseConnector;
use saya_store::SqliteStateStore;
use saya_types::{
    ClaimStatus, Column, ConnectionError, Database, DatabaseProfile, ProfileIdentity, QueryRequest,
    QueryResult, Schema, SchemaTree, SqlDialect, Table,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::agent::learning::turn_table::TurnObjectTable;
use crate::connection::ConnectionEntry;

struct StaticExtractionProvider {
    response_text: String,
    calls: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for StaticExtractionProvider {
    fn name(&self) -> &str {
        "static-provider"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        Ok(ChatResponse::new(ChatMessage::text(
            "assistant",
            &self.response_text,
        )))
    }
}

struct ErrorProvider;

#[async_trait]
impl ChatProvider for ErrorProvider {
    fn name(&self) -> &str {
        "error-provider"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Err(ProviderError::configuration("http 500 server error"))
    }
}

/// A provider that records the one `ChatRequest` `run_extraction` sent, so
/// the JSON-mode intent can be asserted at the call boundary.
struct RecordingProvider {
    response_text: String,
    captured: Mutex<Option<ChatRequest>>,
}

#[async_trait]
impl ChatProvider for RecordingProvider {
    fn name(&self) -> &str {
        "recording-provider"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        *self.captured.lock().unwrap() = Some(request);
        Ok(ChatResponse::new(ChatMessage::text(
            "assistant",
            &self.response_text,
        )))
    }
}

struct IdleConnector;

#[async_trait]
impl DatabaseConnector for IdleConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree {
            databases: vec![Database {
                name: "analytics".into(),
                schemas: vec![Schema {
                    name: "raw".into(),
                    tables: vec![Table {
                        name: "orders".into(),
                        primary_key: vec![],
                        foreign_keys: vec![],
                        columns: vec![Column {
                            name: "status".into(),
                            data_type: "text".into(),
                            nullable: true,
                        }],
                    }],
                }],
            }],
        })
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(req.sql))
    }
}

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-runner-test-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn test_identity(name: &str) -> ProfileIdentity {
    crate::profile_identity::profile_identity(
        name,
        &DatabaseProfile::DuckDb {
            path: "test.duckdb".into(),
            read_only: Some(true),
        },
        Path::new("/test/connections.toml"),
    )
}

fn test_registry(name: &str, identity: &ProfileIdentity) -> ConnectionRegistry {
    let mut reg = ConnectionRegistry::new(name);
    reg.insert(
        name,
        ConnectionEntry {
            connector: Box::new(IdleConnector),
            dialect: SqlDialect::DuckDb,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    reg
}
use crate::agent::learning::TurnRecord;
use crate::connection::ConnectionRegistry;
use crate::contracts::RecallReceipt;

mod execution;
mod request;
mod usage;
