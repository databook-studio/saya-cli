//! Post-turn structured extraction execution runner.

use saya_agent::{
    ChatProvider, ProposedClaimDto, ProviderError, ReasoningEffort, ResponseFormat, TokenUsage,
};
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

/// What a post-turn extraction call produced: the persisted proposals (on
/// success) and the usage the provider reported for the call whenever a
/// response was received. A parse or ingestion failure still carries the
/// usage — tokens may have been billed before the failure — so the accounting
/// is not tied to the success path. A provider error or timeout produces no
/// response, so the usage is `None` and the recorder invents nothing.
pub(crate) struct ExtractionOutcome {
    pub dtos: Result<Vec<ProposedClaimDto>, ExtractionRunnerError>,
    pub usage: Option<TokenUsage>,
}

impl ExtractionOutcome {
    fn ok(dtos: Vec<ProposedClaimDto>, usage: Option<TokenUsage>) -> Self {
        Self {
            dtos: Ok(dtos),
            usage,
        }
    }

    fn failed(error: ExtractionRunnerError, usage: Option<TokenUsage>) -> Self {
        Self {
            dtos: Err(error),
            usage,
        }
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
) -> ExtractionOutcome {
    if record.object_table.is_empty() {
        return ExtractionOutcome::ok(Vec::new(), None);
    }

    let request = build_extraction_prompt(record, model);
    // JSON mode lives here, not in the prompt builder: `build_extraction_prompt`
    // assembles the prompt; the
    // *policy* — "this is the extraction call, so the response must be a single
    // JSON object" — belongs to the caller that knows what the call is for.
    // On a reasoning model this stops the chain-of-thought we never read,
    // cutting the post-turn wait from seconds to ~1s. A provider that
    // cannot honour it degrades to today's behaviour (the prompt already asks
    // for JSON, `strip_markdown_fences` handles fences), never to an error.
    //
    // `Minimal` effort is the honest lever for the same goal — ask for less
    // thinking rather than suppressing it as a side effect of the JSON shape.
    // Both are set together because the measurement shows the effort hint is a
    // no-op on the gateway in use, while JSON mode demonstrably is not: dropping
    // JSON mode would silently restore the multi-second waits, so the mechanism
    // that works stays and the correct lever is added alongside it. Whether the
    // model complied is only knowable from the reported reasoning tokens; saya
    // reports what it asked for, never that the effort was applied.
    let request = request
        .with_response_format(ResponseFormat::JsonObject)
        .with_reasoning_effort(ReasoningEffort::Minimal);
    let response = match provider.complete(request).await {
        Ok(response) => response,
        Err(error) => return ExtractionOutcome::failed(error.into(), None),
    };
    let usage = response.usage;
    let extracted = match parse_extraction_response(&response.message.content, &record.object_table)
    {
        Ok(extracted) => extracted,
        Err(error) => return ExtractionOutcome::failed(error.into(), usage),
    };
    if extracted.is_empty() {
        return ExtractionOutcome::ok(Vec::new(), usage);
    }

    let resolved =
        resolve_proposals(extracted, &record.prompt, &record.object_table, registry).await;

    let filtered = filter_anti_self_reinforcement(resolved, &receipt.supplied);
    if filtered.is_empty() {
        return ExtractionOutcome::ok(Vec::new(), usage);
    }

    let fingerprint = crate::commands::unobserved_fingerprint();
    match ingest_proposals(store, filtered, fingerprint).await {
        Ok(dtos) => ExtractionOutcome::ok(dtos, usage),
        Err(error) => ExtractionOutcome::failed(error.into(), usage),
    }
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

        let outcome = run_extraction(
            &provider,
            "test-model",
            &record,
            &registry,
            &store,
            &receipt,
        )
        .await;

        let res = outcome.dtos.expect("empty table succeeds");

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

        let outcome = run_extraction(
            &provider,
            "test-model",
            &record,
            &registry,
            &store,
            &receipt,
        )
        .await;
        let res = outcome.dtos.expect("extraction succeeds");

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

    /// The model proposes facts about the columns the turn record showed it,
    /// and that list can carry SELECT aliases no table owns. Only the claim
    /// naming a column the object actually has may reach the store.
    #[tokio::test]
    async fn run_extraction_stores_only_column_claims_the_object_actually_has() {
        let identity = test_identity("analytics");
        let registry = test_registry("analytics", &identity);
        let root = temp_root("phantom_column");
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let receipt = RecallReceipt::ran_empty(false);

        let mut object_table = TurnObjectTable::new();
        let t0 = object_table
            .register(
                "analytics",
                "raw.orders",
                &["status".into(), "late_orders".into()],
            )
            .expect("t0 registered");

        let record = TurnRecord {
            prompt: "which orders are late".into(),
            assistant_answer: "late orders are those shipped after the required date".into(),
            object_table,
            user_corrections: Vec::new(),
            override_findings: Vec::new(),
            supplied_claims: Vec::new(),
        };

        let json_payload = format!(
            r#"{{"proposals": [
                {{"object_id": "{t0}", "slot": "column:late_orders.role", "value": "measure", "origin": "assistant_inferred"}},
                {{"object_id": "{t0}", "slot": "column:status.description", "value": "the order state", "origin": "assistant_inferred"}}
            ]}}"#
        );

        let provider = StaticExtractionProvider {
            response_text: json_payload,
            calls: Mutex::new(0),
        };

        let outcome = run_extraction(
            &provider,
            "test-model",
            &record,
            &registry,
            &store,
            &receipt,
        )
        .await;
        let res = outcome.dtos.expect("extraction succeeds");

        assert_eq!(res.len(), 1, "only the claim on the real column is stored");
        assert_eq!(res[0].column.as_deref(), Some("status"));
        assert_eq!(res[0].kind, "column_description");

        let items = store
            .knowledge_for_profile(&identity)
            .await
            .expect("items listed");
        assert_eq!(
            items.len(),
            1,
            "the phantom column claim never reaches the store"
        );
        assert_eq!(
            items[0].slot,
            saya_types::KnowledgeSlot::ColumnDescription {
                column: "status".into()
            }
        );

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
        let outcome = run_extraction(
            &provider,
            "test-model",
            &record,
            &registry,
            &store,
            &receipt,
        )
        .await;

        assert!(matches!(
            outcome.dtos,
            Err(ExtractionRunnerError::Provider(_))
        ));
        let _ = fs::remove_dir_all(root);
    }

    /// The extraction request carries both JSON intent (`ResponseFormat::JsonObject`)
    /// and the honest effort lever (`ReasoningEffort::Minimal`): JSON mode
    /// demonstrably cuts the chain-of-thought on the gateway in use, and Minimal
    /// is the correct lever that works on endpoints which honour it. Both are
    /// set so dropping one cannot silently restore the multi-second waits. This
    /// is the one call in saya that sets them — the main loop's request does not
    /// (see `receive.rs`).
    #[tokio::test]
    async fn run_extraction_sets_json_mode_and_minimal_effort_on_the_provider_request() {
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

        let outcome =
            run_extraction(&provider, "glm-5.2", &record, &registry, &store, &receipt).await;
        let res = outcome.dtos.expect("extraction succeeds");

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
        assert_eq!(
            sent.reasoning_effort,
            ReasoningEffort::Minimal,
            "the extraction call must request minimal effort"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// A provider that returns a response carrying a configurable usage, so the
    /// accounting can assert what the extraction call reported.
    struct UsageExtractionProvider {
        response_text: String,
        usage: TokenUsage,
    }

    #[async_trait]
    impl ChatProvider for UsageExtractionProvider {
        fn name(&self) -> &str {
            "usage-provider"
        }
        async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            let mut response =
                ChatResponse::new(ChatMessage::text("assistant", &self.response_text));
            response.usage = Some(self.usage);
            Ok(response)
        }
    }

    /// A successful extraction surfaces the usage the provider reported, so the
    /// recorder can fold it into the learning total rather than dropping it.
    #[tokio::test]
    async fn run_extraction_surfaces_usage_on_success() {
        let identity = test_identity("analytics");
        let registry = test_registry("analytics", &identity);
        let root = temp_root("usage_ok");
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

        let provider = UsageExtractionProvider {
            response_text: r#"{"proposals": []}"#.into(),
            usage: TokenUsage::new(40, 10),
        };

        let outcome = run_extraction(
            &provider,
            "test-model",
            &record,
            &registry,
            &store,
            &receipt,
        )
        .await;

        outcome.dtos.expect("extraction succeeds");
        assert_eq!(
            outcome.usage,
            Some(TokenUsage::new(40, 10)),
            "a successful extraction must surface the provider-reported usage"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// A failed extraction that still received a response surfaces the usage:
    /// tokens may have been billed before the parse failed, and the accounting
    /// must not be tied to the success path.
    #[tokio::test]
    async fn run_extraction_surfaces_usage_on_parse_failure() {
        let identity = test_identity("analytics");
        let registry = test_registry("analytics", &identity);
        let root = temp_root("usage_parse_err");
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

        let provider = UsageExtractionProvider {
            response_text: "definitely not json".into(),
            usage: TokenUsage::new(40, 10),
        };

        let outcome = run_extraction(
            &provider,
            "test-model",
            &record,
            &registry,
            &store,
            &receipt,
        )
        .await;

        assert!(
            matches!(outcome.dtos, Err(ExtractionRunnerError::Extraction(_))),
            "malformed content must fail extraction, got: {:?}",
            outcome.dtos
        );
        assert_eq!(
            outcome.usage,
            Some(TokenUsage::new(40, 10)),
            "a failed extraction that received a response must still surface its usage"
        );
        let _ = fs::remove_dir_all(root);
    }
}
