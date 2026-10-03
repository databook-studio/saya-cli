//! Proves the real runtime wires the config context budget onto the loop.
//!
//! The env-driven ceilings (turns, tool calls, continuations) are proven at
//! the real binary boundary instead — `crates/saya-cli/tests/agent_budgets.rs`
//! spawns `saya ask` with the ceiling variables planted on the child
//! `Command` only, so no test in this process ever mutates the process-global
//! environment the runtime reads at `runtime.rs:270`. What stays here needs
//! no env mutation: the config `context_byte_budget` is threaded through the
//! real turn path and observed in the history the loop sends the provider.

use super::super::turn_inputs::TurnInputs;
use super::{AgentRuntimeError, run_prompt_with_inputs};
use crate::config::runtime::RuntimeConfig;
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentMode, AgentOutput, ApprovalPolicy, CancellationToken,
    ChatMessage, ChatProvider, ChatRequest, ChatResponse, NoopEventSink,
};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_types::{
    Column, ConnectionError, Database, DatabaseProfile, ProfileIdentity, QueryRequest, QueryResult,
    Schema, SchemaTree, SqlDialect, Table,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// A connector that never touches a live database: the probe tool calls run
/// against it, and no test here reads the store.
struct IdleConnector;

#[async_trait]
impl saya_connectors::DatabaseConnector for IdleConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree {
            databases: vec![Database {
                name: "catalog".into(),
                schemas: vec![Schema {
                    name: "public".into(),
                    tables: vec![orders_table()],
                }],
            }],
        })
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(req.sql))
    }
}

fn orders_table() -> Table {
    Table {
        name: "orders".into(),
        columns: vec![
            Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            },
            Column {
                name: "created_at".into(),
                data_type: "timestamp".into(),
                nullable: false,
            },
        ],
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

fn registry_for(name: &str) -> ConnectionRegistry {
    let identity: ProfileIdentity = crate::profile_identity::profile_identity(
        name,
        &DatabaseProfile::DuckDb {
            path: "runtime-budget.duckdb".into(),
            read_only: Some(true),
        },
        Path::new("/runtime-budget-test/connections.toml"),
    );
    let mut registry = ConnectionRegistry::new(name);
    registry.insert(
        name,
        ConnectionEntry {
            connector: Box::new(IdleConnector),
            dialect: SqlDialect::DuckDb,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    registry
}

fn test_ai(context_byte_budget: usize) -> ResolvedAi {
    ResolvedAi {
        provider: AiProvider::Ollama,
        model: "test-model".into(),
        base_url: None,
        api_key: None,
        allow_data_sharing: true,
        temperature: 0.0,
        timeout_seconds: 60,
        idle_timeout_seconds: 90,
        max_output_tokens: 4096,
        max_output_tokens_is_default: true,
        context_byte_budget,
        context_window_tokens: None,
        show_thinking: false,
        compaction: saya_config::CompactionMode::Auto,
        retry_delays_ms: vec![250, 500, 1000],
    }
}

/// A minimal `RuntimeConfig` carrying only what the turn reads. The provider
/// and registry arrive via `TurnInputs`; memory is off, so recall and the
/// post-turn extraction are skipped and only the loop's limits are in play.
fn test_runtime(context_byte_budget: usize) -> RuntimeConfig {
    RuntimeConfig {
        resolved: ResolvedConfig {
            profile_name: None,
            profile: None,
            ai: test_ai(context_byte_budget),
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            candidates: 1,
            jobs: ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: BTreeMap::new(),
                turns: Some(4),
                tool_calls: None,
                fetch: ResolvedFetchJobs::default(),
                interpreter: ResolvedInterpreterJobs::default(),
                runner: ResolvedRunnerJobs::default(),
            },
            query_timeout_seconds: 5,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Auto,
            ui_theme: ThemeChoice::Auto,
            memory: ResolvedMemory {
                mode: MemoryMode::Off,
                max_contracts: 5,
                max_claims_per_contract: 12,
                max_context_bytes: 16384,
            },
            host_commands: ResolvedHostCommands::default(),
            session_deny: Default::default(),
            ignored_project_overrides: Vec::new(),
            endpoints: BTreeMap::new(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-runtime-budget-tests"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: BTreeMap::new(),
    }
}

/// The messages of the first provider call: the history the loop assembled
/// under the config `context_byte_budget`.
#[derive(Default)]
struct Counters {
    first_answering_request: Mutex<Option<ChatRequest>>,
    request_count: Mutex<usize>,
}

impl Counters {
    fn first_messages(&self) -> Option<Vec<ChatMessage>> {
        self.first_request().map(|request| request.messages)
    }

    fn first_request(&self) -> Option<ChatRequest> {
        self.first_answering_request.lock().unwrap().clone()
    }

    fn request_count(&self) -> usize {
        *self.request_count.lock().unwrap()
    }
}

/// A provider that answers every call with plain text, recording the first
/// request's messages.
struct AnswerProbe {
    counters: Arc<Counters>,
}

#[async_trait]
impl ChatProvider for AnswerProbe {
    fn name(&self) -> &str {
        "budget-probe"
    }
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        *self.counters.request_count.lock().unwrap() += 1;
        {
            let mut first = self.counters.first_answering_request.lock().unwrap();
            if first.is_none() {
                *first = Some(request.clone());
            }
        }
        Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
    }
}

/// Runs one turn through the real runtime path, returning the result and the
/// probe counters.
async fn run_turn(
    context_byte_budget: usize,
    history: Vec<ChatMessage>,
) -> (Result<AgentOutput, AgentRuntimeError>, Arc<Counters>) {
    let sink = NoopEventSink;
    run_turn_with_request(
        context_byte_budget,
        "orders by month",
        history,
        None,
        test_ai(context_byte_budget),
        "analytics",
        &sink,
    )
    .await
}

async fn run_turn_with_request(
    context_byte_budget: usize,
    prompt: &str,
    history: Vec<ChatMessage>,
    last_sql: Option<&str>,
    ai: ResolvedAi,
    profile: &str,
    sink: &dyn AgentEventSink,
) -> (Result<AgentOutput, AgentRuntimeError>, Arc<Counters>) {
    let counters = Arc::new(Counters::default());
    let inputs = TurnInputs {
        ai,
        provider: Box::new(AnswerProbe {
            counters: Arc::clone(&counters),
        }),
        registry: registry_for(profile),
        failures: Vec::new(),
    };
    let runtime = test_runtime(context_byte_budget);
    let result = run_prompt_with_inputs(
        &runtime,
        inputs,
        prompt,
        ApprovalPolicy::ReadOnly,
        false,
        false,
        history,
        sink,
        CancellationToken::new(),
        None,
        None,
        last_sql.map(str::to_owned),
        None,
        AgentMode::Build,
        None,
    )
    .await;
    (result, counters)
}

async fn record_first_request(prompt: &str, last_sql: Option<&str>) -> ChatRequest {
    record_first_request_for(
        prompt,
        last_sql,
        test_ai(16 * 1024),
        "analytics",
        &NoopEventSink,
    )
    .await
}

async fn record_first_request_for(
    prompt: &str,
    last_sql: Option<&str>,
    ai: ResolvedAi,
    profile: &str,
    sink: &dyn AgentEventSink,
) -> ChatRequest {
    let (result, counters) =
        run_turn_with_request(16 * 1024, prompt, Vec::new(), last_sql, ai, profile, sink).await;
    result.expect("the turn completes");
    counters
        .first_request()
        .expect("the provider receives the runtime's first complete request")
}

struct TimedSink {
    started: Instant,
    events: Mutex<Vec<AgentEvent>>,
    first_assistant_text: Mutex<Option<Duration>>,
}

impl TimedSink {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            events: Mutex::new(Vec::new()),
            first_assistant_text: Mutex::new(None),
        }
    }

    fn first_assistant_text(&self) -> Option<Duration> {
        *self.first_assistant_text.lock().unwrap()
    }

    fn event_count(&self, matches: fn(&AgentEvent) -> bool) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches(event))
            .count()
    }
}

#[async_trait]
impl AgentEventSink for TimedSink {
    async fn emit(&self, event: AgentEvent) {
        if matches!(&event, AgentEvent::AssistantText { text } if !text.is_empty()) {
            let mut first = self.first_assistant_text.lock().unwrap();
            if first.is_none() {
                *first = Some(self.started.elapsed());
            }
        }
        self.events.lock().unwrap().push(event);
    }
}

struct RequestMeasurement {
    request: ChatRequest,
    request_count: usize,
    tool_calls: usize,
    retries: usize,
    elapsed: Duration,
    first_assistant_text: Option<Duration>,
}

async fn capture_repeated_requests() -> Vec<RequestMeasurement> {
    let mut measurements = Vec::new();
    for _ in 0..5 {
        let sink = TimedSink::new();
        let (result, counters) = run_turn_with_request(
            16 * 1024,
            "orders by month",
            Vec::new(),
            Some("SELECT id FROM orders"),
            test_ai(16 * 1024),
            "analytics",
            &sink,
        )
        .await;
        result.expect("the fixed local turn completes");
        measurements.push(RequestMeasurement {
            request: counters.first_request().expect("a request was captured"),
            request_count: counters.request_count(),
            tool_calls: sink.event_count(|event| matches!(event, AgentEvent::ToolRequested { .. })),
            retries: sink.event_count(|event| matches!(event, AgentEvent::TurnReset)),
            elapsed: sink.started.elapsed(),
            first_assistant_text: sink.first_assistant_text(),
        });
    }
    measurements
}

// ===========================================================================
// The config context budget: `ai.context_byte_budget` is distinct from the
// built-in default (256 KiB, which would replay the oversized oldest pair)
// and from the harness's `max_rows` (100, which would replay nothing at all),
// so each wrong composition of `AgentLimits::context_byte_budget` is
// distinguishable from the right one by what the loop sent the provider.
// ===========================================================================

#[tokio::test]
async fn config_context_byte_budget_bounds_the_history_the_loop_sends() {
    let history = vec![
        ChatMessage::text(
            "user",
            format!("OLD-PAIR-SENTINEL-{}", "x".repeat(64 * 1024)),
        ),
        ChatMessage::text("assistant", "old answer"),
        ChatMessage::text("user", "NEW-PAIR-SENTINEL"),
        ChatMessage::text("assistant", "new answer"),
    ];
    let (result, counters) = run_turn(16 * 1024, history).await;
    result.expect("the turn completes");
    let messages = counters
        .first_messages()
        .expect("the first request was recorded");
    assert_eq!(
        messages.len(),
        4,
        "system + the one fitting pair + the current user turn: {messages:?}"
    );
    let joined = messages
        .iter()
        .map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("NEW-PAIR-SENTINEL"),
        "the newest pair replays under the planted budget: {joined}"
    );
    assert!(
        !joined.contains("OLD-PAIR-SENTINEL"),
        "the oversized oldest pair is trimmed by the planted budget: {joined}"
    );
}

#[tokio::test]
async fn actual_runtime_request_prefix_is_stable_and_last_sql_stays_in_the_user_tail() {
    let measurements = capture_repeated_requests().await;
    assert_eq!(measurements.len(), 5, "five fixed local runs were captured");
    let first = &measurements[0];
    let request_bytes = serde_json::to_vec(&first.request).unwrap().len();
    let system_bytes =
        first.request.messages[0].role.len() + first.request.messages[0].content.len();
    let message_bytes = first
        .request
        .messages
        .iter()
        .map(|message| message.role.len() + message.content.len())
        .sum::<usize>();
    let tool_bytes = serde_json::to_vec(&first.request.tools).unwrap().len();
    let request = serde_json::to_vec(&first.request).unwrap();
    assert!(
        measurements
            .iter()
            .all(|measurement| serde_json::to_vec(&measurement.request).unwrap() == request),
        "fixed runtime inputs must produce the same complete request on all five runs"
    );
    assert!(
        measurements.iter().all(|measurement| {
            measurement.request_count == 1
                && measurement.tool_calls == 0
                && measurement.retries == 0
        }),
        "the fixed provider made one request with no tool calls or retries"
    );
    assert!(
        measurements
            .iter()
            .all(|measurement| measurement.first_assistant_text.is_some()),
        "each fixed run emitted nonempty assistant text through the event sink"
    );
    let changed = record_first_request(
        "orders by customer",
        Some("SELECT missing FROM missing_table"),
    )
    .await;
    let non_ascii = record_first_request("orders in München 😀", None).await;
    let profile_changed = record_first_request_for(
        "orders by month",
        None,
        test_ai(16 * 1024),
        "warehouse",
        &NoopEventSink,
    )
    .await;
    assert_eq!(
        serde_json::to_vec(&first.request.tools).unwrap(),
        serde_json::to_vec(&changed.tools).unwrap(),
        "question and last SQL cannot reorder or alter tool schemas"
    );
    assert_eq!(
        first.request.messages[0], changed.messages[0],
        "per-turn inputs must not alter the system prefix"
    );
    assert!(
        changed
            .messages
            .last()
            .unwrap()
            .content
            .contains("orders by customer")
    );
    assert!(
        changed
            .messages
            .last()
            .unwrap()
            .content
            .contains("SELECT missing FROM missing_table")
    );
    assert_eq!(
        first.request.tools.len(),
        non_ascii.tools.len(),
        "the full tool set stays attached"
    );
    assert!(
        non_ascii
            .messages
            .last()
            .unwrap()
            .content
            .contains("München 😀")
    );
    assert_ne!(
        first.request.messages[0], profile_changed.messages[0],
        "the selected profile changes the production-built session context"
    );
    assert!(profile_changed.messages[0].content.contains("warehouse"));
    let elapsed = measurements.iter().map(|measurement| measurement.elapsed);
    let first_text = measurements
        .iter()
        .map(|measurement| measurement.first_assistant_text.unwrap());
    eprintln!(
        "request corpus local fixed-provider: runs=5 request_bytes={request_bytes} \
         system_bytes={system_bytes} messages_bytes={message_bytes} tools_bytes={tool_bytes} \
         requests=1 tool_calls=0 retries=0 elapsed_us={}..{} first_assistant_text_us={}..{}",
        elapsed.clone().min().unwrap().as_micros(),
        elapsed.max().unwrap().as_micros(),
        first_text.clone().min().unwrap().as_micros(),
        first_text.max().unwrap().as_micros(),
    );
}
