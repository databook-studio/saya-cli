//! The harness the plugin's `on_turn_finish` tests run over: a scratch store,
//! one observed object carrying the turn's evidence, and a scripted provider
//! whose extractor reply proposes one claim.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentOutput, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    ProviderError, TokenUsage, TurnOutcome,
};
use saya_connectors::DatabaseConnector;
use saya_store::SqliteStateStore;
use saya_types::{
    Column, ConnectionError, Database, DatabaseProfile, ProfileIdentity, QueryRequest, QueryResult,
    Schema, SchemaTree, SqlDialect, Table,
};

use super::LearningSource;
use crate::agent::tools::{
    DatabaseTools, ObservationLog, ObservationOutcome, OverrideLog, ToolObservation,
};
use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// The question this turn answered.
pub(super) const PROMPT: &str = "what does the orders status column mean";
/// The model's answer. Long enough that the gate treats the turn as worth
/// extracting from, so a test can tell "the gate declined" from "the reply
/// proposed nothing".
pub(super) const ANSWER: &str = "status is the order state recorded at checkout";
pub(super) const MODEL: &str = "scripted-extractor";

/// The extractor's reply: one claim about the `status` column of the only
/// object the turn observed (`T0`, the first id a turn's object table assigns).
const PAYLOAD: &str = r#"{"proposals": [{"object_id": "T0", "slot": "column:status.description", "value": "the order state", "origin": "assistant_inferred"}]}"#;

/// A provider that answers every extraction call with the same one-claim reply
/// and reports usage, counting the calls it received.
pub(super) struct ScriptedExtraction {
    calls: Mutex<usize>,
}

impl ScriptedExtraction {
    pub(super) fn new() -> Self {
        Self {
            calls: Mutex::new(0),
        }
    }

    /// How many provider requests arrived — zero means the turn never asked.
    pub(super) fn calls(&self) -> usize {
        *self.calls.lock().expect("call counter")
    }
}

#[async_trait]
impl ChatProvider for ScriptedExtraction {
    fn name(&self) -> &str {
        "scripted-extraction"
    }

    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        *self.calls.lock().expect("call counter") += 1;
        let mut response = ChatResponse::new(ChatMessage::text("assistant", PAYLOAD));
        response.usage = Some(TokenUsage::new(4, 2));
        Ok(response)
    }
}

/// A connector that never touches a live database: extraction reads the turn's
/// record and writes the store, so the registry only needs an entry carrying an
/// identity — and, because a proposal is resolved against the profile's real
/// schema, the one table the turn observed.
struct IdleConnector;

#[async_trait]
impl DatabaseConnector for IdleConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Sqlite
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

/// A scratch directory under the workspace's own `target/` — never the system
/// temp directory.
pub(super) fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-plugins-database-learning-{name}"));
    std::fs::create_dir_all(&root).expect("scratch directory");
    root
}

pub(super) fn identity() -> ProfileIdentity {
    crate::profile_identity::profile_identity(
        "analytics",
        &DatabaseProfile::Sqlite {
            path: "plugin-learning.sqlite".into(),
            read_only: true,
        },
        Path::new("/plugin-learning-test/connections.toml"),
    )
}

/// A registry whose single primary connection carries the identity the turn's
/// observations and proposals resolve against.
pub(super) fn registry(identity: &ProfileIdentity) -> ConnectionRegistry {
    let mut registry = ConnectionRegistry::new("analytics");
    registry.insert(
        "analytics",
        ConnectionEntry {
            connector: Box::new(IdleConnector),
            dialect: SqlDialect::Sqlite,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    registry
}

/// An observation log carrying the one query the turn ran: it touched
/// `raw.orders` and read `status`.
pub(super) fn observations() -> Arc<ObservationLog> {
    let log = ObservationLog::new();
    log.record(ToolObservation {
        tool: "bounded_sql_query".into(),
        outcome: ObservationOutcome::Succeeded,
        profile: None,
        objects: vec![vec!["raw".into(), "orders".into()]],
        columns: vec!["status".into()],
        row_count: Some(5),
        truncated: Some(false),
        references_partial: false,
    });
    Arc::new(log)
}

/// The database tools the extraction call reads its store and registry from.
pub(super) fn tools(
    store: &Path,
    identity: &ProfileIdentity,
    observations: Arc<ObservationLog>,
) -> Arc<DatabaseTools> {
    Arc::new(DatabaseTools::with_learning(
        registry(identity),
        100,
        true,
        Some(SqliteStateStore::new(store)),
        Some(observations),
    ))
}

/// The chat path's answer for the turn — what extraction reads.
pub(super) fn output() -> AgentOutput {
    AgentOutput {
        answer: ANSWER.into(),
        events: Vec::new(),
        used_bounded_sql_query: true,
        tool_metadata: Vec::new(),
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

/// The hook's outcome for an answered turn whose output is `output`.
pub(super) fn finished<'a>(output: Option<&'a AgentOutput>) -> TurnOutcome<'a> {
    TurnOutcome {
        answered: true,
        cancelled: false,
        prompt: PROMPT,
        output,
    }
}

/// The learning source a run attaches when it configured memory: the chat's
/// own turn inputs, minus the prompt and output, which only the hook has.
pub(super) fn source(
    tools: &Arc<DatabaseTools>,
    observations: Arc<ObservationLog>,
    provider: Arc<dyn ChatProvider>,
) -> LearningSource {
    LearningSource::new(
        true,
        tools.clone(),
        Some(observations),
        Arc::new(OverrideLog::new()),
        provider,
        MODEL,
    )
}

/// A sink that keeps every event it is handed, so a test can compare what a
/// hook emitted with what the shared function emits for the same inputs.
#[derive(Default)]
pub(super) struct RecordingSink {
    events: Mutex<Vec<AgentEvent>>,
}

impl RecordingSink {
    pub(super) fn events(&self) -> Vec<AgentEvent> {
        self.events.lock().expect("event log").clone()
    }
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().expect("event log").push(event);
    }
}
