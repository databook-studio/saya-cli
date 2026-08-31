//! Post-turn structured extraction execution runner — spec F Chunk 4.

use saya_agent::{ChatProvider, ChatRequest, ProposedClaimDto, ProviderError, ResponseFormat};
use saya_store::KnowledgeItemStore;
use std::fmt;

use super::{
    ExtractionError, IngestionError, TurnRecord, build_extraction_prompt,
    filter_anti_self_reinforcement, ingest_proposals, parse_extraction_response, resolve_proposals,
};
use crate::connection::ConnectionRegistry;
use crate::contracts::RecallReceipt;

/// Errors that may occur during post-turn extraction.
#[derive(Debug)]
pub(crate) enum ExtractionRunnerError {
    Provider(ProviderError),
    Extraction(ExtractionError),
    Ingestion(IngestionError),
}

impl fmt::Display for ExtractionRunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provider(e) => write!(f, "extraction provider error: {e:?}"),
            Self::Extraction(e) => write!(f, "extraction parse error: {e}"),
            Self::Ingestion(e) => write!(f, "extraction ingestion error: {e}"),
        }
    }
}

impl std::error::Error for ExtractionRunnerError {}

impl From<ProviderError> for ExtractionRunnerError {
    fn from(e: ProviderError) -> Self {
        Self::Provider(e)
    }
}

impl From<ExtractionError> for ExtractionRunnerError {
    fn from(e: ExtractionError) -> Self {
        Self::Extraction(e)
    }
}

impl From<IngestionError> for ExtractionRunnerError {
    fn from(e: IngestionError) -> Self {
        Self::Ingestion(e)
    }
}

/// Executes post-turn structured extraction using the given provider and persists resolved proposals.
pub(crate) async fn run_extraction(
    provider: &dyn ChatProvider,
    model: &str,
    record: &TurnRecord,
    registry: &ConnectionRegistry,
    store: &dyn KnowledgeItemStore,
    receipt: &RecallReceipt,
) -> Result<Vec<ProposedClaimDto>, ExtractionRunnerError> {
    if record.object_table.is_empty() {
        return Ok(Vec::new());
    }

    let request = build_extraction_prompt(record, model);
    // JSON mode lives here, not in the prompt builder: `build_extraction_prompt`
    // assembles the prompt (its text is out of scope for this slice); the
    // *policy* — "this is the extraction call, so the response must be a single
    // JSON object" — belongs to the caller that knows what the call is for.
    // On a reasoning model this stops the chain-of-thought we never read,
    // cutting the post-turn wait from seconds to ~1s (spec S19). A provider that
    // cannot honour it degrades to today's behaviour (the prompt already asks
    // for JSON, `strip_markdown_fences` handles fences), never to an error.
    let request = ChatRequest {
        response_format: ResponseFormat::JsonObject,
        ..request
    };
    let response = provider.complete(request).await?;
    let extracted = parse_extraction_response(&response.message.content, &record.object_table)?;
    if extracted.is_empty() {
        return Ok(Vec::new());
    }

    let resolved = resolve_proposals(extracted, &record.object_table, registry).await;

    let filtered = filter_anti_self_reinforcement(resolved, &receipt.supplied);
    if filtered.is_empty() {
        return Ok(Vec::new());
    }

    let fingerprint = crate::commands::unobserved_fingerprint();
    let dtos = ingest_proposals(store, filtered, fingerprint).await?;
    Ok(dtos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use saya_agent::ChatResponse;
    use saya_agent::{ChatMessage, ChatRequest};
    use saya_connectors::DatabaseConnector;
    use saya_store::SqliteStateStore;
    use saya_types::{
        ClaimStatus, Column, ConnectionError, Database, DatabaseProfile, ProfileIdentity,
        QueryRequest, QueryResult, Schema, SchemaTree, SqlDialect, Table,
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
            Ok(ChatResponse {
                message: ChatMessage::text("assistant", &self.response_text),
            })
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
    /// the JSON-mode intent can be asserted at the call boundary (deliverable 4).
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
            Ok(ChatResponse {
                message: ChatMessage::text("assistant", &self.response_text),
            })
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

    #[tokio::test]
    async fn run_extraction_empty_object_table_skips_provider() {
        let identity = test_identity("analytics");
        let registry = test_registry("analytics", &identity);
        let root = temp_root("empty_table");
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let receipt = RecallReceipt::ran_empty(false);

        let record = TurnRecord {
            prompt: "hello".into(),
            assistant_answer: "hi".into(),
            object_table: TurnObjectTable::new(),
            user_corrections: Vec::new(),
            override_findings: Vec::new(),
            supplied_claims: Vec::new(),
        };

        let provider = StaticExtractionProvider {
            response_text: r#"{"proposals": []}"#.into(),
            calls: Mutex::new(0),
        };

        let res = run_extraction(
            &provider,
            "test-model",
            &record,
            &registry,
            &store,
            &receipt,
        )
        .await
        .expect("empty table succeeds");

        assert!(res.is_empty());
        assert_eq!(*provider.calls.lock().unwrap(), 0);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn run_extraction_success_stores_and_returns_dtos() {
        let identity = test_identity("analytics");
        let registry = test_registry("analytics", &identity);
        let root = temp_root("success");
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let receipt = RecallReceipt::ran_empty(false);

        let mut object_table = TurnObjectTable::new();
        let t0 = object_table
            .register("analytics", "raw.orders", &["status".into()])
            .expect("t0 registered");

        let record = TurnRecord {
            prompt: "what is orders status".into(),
            assistant_answer: "status is pending".into(),
            object_table,
            user_corrections: Vec::new(),
            override_findings: Vec::new(),
            supplied_claims: Vec::new(),
        };

        let json_payload = format!(
            r#"{{"proposals": [{{"object_id": "{t0}", "slot": "column:status.description", "value": "status is order state", "origin": "assistant_inferred"}}]}}"#
        );

        let provider = StaticExtractionProvider {
            response_text: json_payload,
            calls: Mutex::new(0),
        };

        let res = run_extraction(
            &provider,
            "test-model",
            &record,
            &registry,
            &store,
            &receipt,
        )
        .await
        .expect("extraction succeeds");

        assert_eq!(res.len(), 1);
        assert_eq!(res[0].profile, "analytics");
        // Object identity is fully qualified: a bare `raw.orders` would be
        // ambiguous across catalogs.
        assert_eq!(res[0].object, "analytics.raw.orders");
        assert_eq!(res[0].column.as_deref(), Some("status"));
        assert_eq!(res[0].status, ClaimStatus::Candidate);

        // Ingestion writes to `knowledge_items` (D-3's projection), not the legacy
        // `contract_claims` table — asserting against the old store would pass only
        // if F had quietly kept writing to the thing D-3 replaced.
        let items = store
            .knowledge_for_profile(&identity)
            .await
            .expect("items listed");
        assert_eq!(items.len(), 1);

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn run_extraction_propagates_provider_error() {
        let identity = test_identity("analytics");
        let registry = test_registry("analytics", &identity);
        let root = temp_root("err_prop");
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let receipt = RecallReceipt::ran_empty(false);

        let mut object_table = TurnObjectTable::new();
        object_table.register("analytics", "raw.orders", &["status".into()]);

        let record = TurnRecord {
            prompt: "test".into(),
            assistant_answer: "ans".into(),
            object_table,
            user_corrections: Vec::new(),
            override_findings: Vec::new(),
            supplied_claims: Vec::new(),
        };

        let provider = ErrorProvider;
        let res = run_extraction(
            &provider,
            "test-model",
            &record,
            &registry,
            &store,
            &receipt,
        )
        .await;

        assert!(matches!(res, Err(ExtractionRunnerError::Provider(_))));
        let _ = fs::remove_dir_all(root);
    }

    /// Deliverable 4: the request `run_extraction` sends to the provider carries
    /// the JSON intent (`ResponseFormat::JsonObject`), so a reasoning model skips
    /// the chain-of-thought we discard. This is the one call in saya that sets
    /// it — the main loop's request does not (see `receive.rs`).
    #[tokio::test]
    async fn run_extraction_sets_json_mode_on_the_provider_request() {
        let identity = test_identity("analytics");
        let registry = test_registry("analytics", &identity);
        let root = temp_root("json_mode");
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let receipt = RecallReceipt::ran_empty(false);

        let mut object_table = TurnObjectTable::new();
        object_table.register("analytics", "raw.orders", &["status".into()]);

        let record = TurnRecord {
            prompt: "what is orders status".into(),
            assistant_answer: "status is pending".into(),
            object_table,
            user_corrections: Vec::new(),
            override_findings: Vec::new(),
            supplied_claims: Vec::new(),
        };

        let provider = RecordingProvider {
            response_text: r#"{"proposals": []}"#.into(),
            captured: Mutex::new(None),
        };

        let res = run_extraction(&provider, "glm-5.2", &record, &registry, &store, &receipt)
            .await
            .expect("extraction succeeds");

        assert!(res.is_empty(), "empty-proposal response stores nothing");
        let sent = provider
            .captured
            .lock()
            .unwrap()
            .take()
            .expect("a request was sent to the provider");
        assert_eq!(
            sent.response_format,
            ResponseFormat::JsonObject,
            "the extraction call must request JSON mode"
        );
        let _ = fs::remove_dir_all(root);
    }
}
