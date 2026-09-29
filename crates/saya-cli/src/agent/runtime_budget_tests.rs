//! Proves the real runtime wires every budget ceiling onto the agent limits.
//!
//! `budgets_from_env` parsing has its own tests in `saya-agent`; what they
//! cannot see is the composition [`super::run_prompt_with_inputs`] performs:
//! which parsed value lands on which `AgentLimits` field. These tests plant
//! DISTINCT ceiling values and run a scripted provider through the real turn
//! path, observing the limits at the loop/request boundary — where the loop
//! stops (turn and tool-call ceilings), how many truncation retries it spends
//! (continuation ceiling), and how much history the loop sends (the config
//! `context_byte_budget`). A dropped or swapped assignment moves one of those
//! observables.
//!
//! The runtime reads the ceiling variables through `std::env::var`, so the
//! tests plant real process-global variables under [`ENV_LOCK`] — the
//! discipline `tests/run_slash_parity.rs` uses — and restore them afterwards.

use super::super::turn_inputs::TurnInputs;
use super::{AgentRuntimeError, run_prompt_with_inputs};
use crate::config::runtime::RuntimeConfig;
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use async_trait::async_trait;
use saya_agent::{
    AgentMode, AgentOutput, ApprovalPolicy, CancellationToken, ChatMessage, ChatProvider,
    ChatRequest, ChatResponse, NoopEventSink, ProviderError, ToolCall,
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
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// The ceiling variables the runtime composes onto `AgentLimits` (the names
/// `budgets_from_env` reads; kept here as the test's env protocol).
const BUDGET_VARS: [&str; 3] = [
    "SAYA_AGENT_MAX_TURNS",
    "SAYA_AGENT_MAX_TOOL_CALLS",
    "SAYA_AGENT_MAX_CONTINUATIONS",
];

/// The env lock: ceiling variables are process-global and the tests in this
/// binary run concurrently. Every test here takes this lock for its whole
/// body — across awaits, deliberately — so no other test observes a torn or
/// foreign value. Tokio's `Mutex` because a std guard held across an await is
/// exactly the deadlock clippy names.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Plants (or clears) the ceiling variables, saving every variable's previous
/// state for [`restore_budgets`]. Callers list all three variables explicitly
/// so each test's regime is stated in full.
///
/// SAFETY: `set_var`/`remove_var` mutate process-global state; the caller
/// holds [`ENV_LOCK`] for the whole test body, so no other test in this binary
/// observes a torn or foreign value.
fn plant_budgets(values: &[(&str, Option<&str>)]) -> Vec<(&'static str, Option<OsString>)> {
    let saved = BUDGET_VARS.map(|name| (name, std::env::var_os(name)));
    for (name, value) in values {
        match value {
            Some(value) => {
                // SAFETY: the caller holds ENV_LOCK for the whole test body.
                unsafe { std::env::set_var(name, value) };
            }
            None => {
                // SAFETY: the caller holds ENV_LOCK for the whole test body.
                unsafe { std::env::remove_var(name) };
            }
        }
    }
    saved.to_vec()
}

/// Restores the state [`plant_budgets`] saved.
///
/// SAFETY: same lock discipline as [`plant_budgets`].
fn restore_budgets(saved: Vec<(&'static str, Option<OsString>)>) {
    for (name, value) in saved {
        match value {
            Some(value) => {
                // SAFETY: the caller holds ENV_LOCK for the whole test body.
                unsafe { std::env::set_var(name, value) };
            }
            None => {
                // SAFETY: the caller holds ENV_LOCK for the whole test body.
                unsafe { std::env::remove_var(name) };
            }
        }
    }
}

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

/// What the scripted provider answers, and the counters it keeps — the
/// observation point for what the loop did with the limits it was handed.
#[derive(Default)]
struct Counters {
    /// Provider calls that carried tool definitions — the loop's answering
    /// turns (and the turn whose batch the tool-call ceiling refuses).
    answering: Mutex<usize>,
    /// Provider calls that carried no tool definitions — the ceiling salvage
    /// call (the loop strips the definitions there).
    salvage: Mutex<usize>,
    /// The messages of the first answering call: the history the loop
    /// assembled under the config `context_byte_budget`.
    first_answering_messages: Mutex<Option<Vec<ChatMessage>>>,
}

impl Counters {
    fn answering(&self) -> usize {
        *self.answering.lock().unwrap()
    }
    fn salvage(&self) -> usize {
        *self.salvage.lock().unwrap()
    }
    fn first_messages(&self) -> Option<Vec<ChatMessage>> {
        self.first_answering_messages.lock().unwrap().clone()
    }
}

/// The provider's script.
enum ProbeScript {
    /// Each answering call up to `calls` (counted from one) returns one
    /// `bounded_sql_query` tool call; later calls return a plain text answer.
    /// Self-terminating: a dropped turn or tool-call ceiling degrades to a
    /// completed run instead of hanging the test.
    ToolCalls { calls: usize },
    /// Every call answers with plain text.
    Answer,
    /// Every call fails with the provider's output-token truncation signal —
    /// the deterministic cap the continuation ceiling bounds.
    AlwaysTruncated,
}

struct BudgetProbe {
    script: ProbeScript,
    counters: Arc<Counters>,
}

#[async_trait]
impl ChatProvider for BudgetProbe {
    fn name(&self) -> &str {
        "budget-probe"
    }
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        // The salvage call is the one request that carries no tool
        // definitions: the loop strips them there. Counting on the request
        // shape keeps the oracle structural, not tied to a product string.
        if request.tools.is_empty() {
            *self.counters.salvage.lock().unwrap() += 1;
            return Ok(ChatResponse::new(ChatMessage::text(
                "assistant",
                "salvaged answer",
            )));
        }
        let call = {
            let mut answering = self.counters.answering.lock().unwrap();
            *answering += 1;
            *answering
        };
        {
            let mut first = self.counters.first_answering_messages.lock().unwrap();
            if first.is_none() {
                *first = Some(request.messages.clone());
            }
        }
        match self.script {
            ProbeScript::AlwaysTruncated => Err(ProviderError::output_truncated(
                "partial answer cut off".into(),
                Vec::new(),
            )),
            ProbeScript::ToolCalls { calls } if call <= calls => {
                Ok(ChatResponse::new(ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: format!("call-{call}"),
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({
                            "connection": "analytics",
                            "sql": "SELECT id FROM orders",
                        }),
                    }],
                    tool_call_id: None,
                }))
            }
            ProbeScript::ToolCalls { .. } | ProbeScript::Answer => {
                Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
            }
        }
    }
}

/// Runs one turn through the real runtime path with the planted ceilings,
/// returning the result and the probe counters.
async fn run_turn(
    context_byte_budget: usize,
    script: ProbeScript,
    history: Vec<ChatMessage>,
) -> (Result<AgentOutput, AgentRuntimeError>, Arc<Counters>) {
    let counters = Arc::new(Counters::default());
    let inputs = TurnInputs {
        ai: test_ai(context_byte_budget),
        provider: Box::new(BudgetProbe {
            script,
            counters: Arc::clone(&counters),
        }),
        registry: registry_for("analytics"),
        failures: Vec::new(),
    };
    let runtime = test_runtime(context_byte_budget);
    let result = run_prompt_with_inputs(
        &runtime,
        inputs,
        "orders by month",
        ApprovalPolicy::ReadOnly,
        false,
        false,
        history,
        &NoopEventSink,
        CancellationToken::new(),
        None,
        None,
        None,
        None,
        AgentMode::Build,
        None,
    )
    .await;
    (result, counters)
}

// ===========================================================================
// The turn ceiling: SAYA_AGENT_MAX_TURNS=3 must stop the loop after exactly
// three provider turns with tools, via the ceiling salvage — proving the
// env-parsed value is what the loop actually received.
// ===========================================================================

#[tokio::test]
async fn env_turn_ceiling_stops_the_loop_at_the_planted_turn_count() {
    let _env = ENV_LOCK.lock().await;
    let saved = plant_budgets(&[
        ("SAYA_AGENT_MAX_TURNS", Some("3")),
        ("SAYA_AGENT_MAX_TOOL_CALLS", None),
        ("SAYA_AGENT_MAX_CONTINUATIONS", None),
    ]);
    // Twelve scripted tool-call responses: the planted ceiling binds first.
    // A dropped assignment runs to the script's end and completes untruncated,
    // so the test goes red without hanging.
    let (result, counters) =
        run_turn(256 * 1024, ProbeScript::ToolCalls { calls: 12 }, Vec::new()).await;
    restore_budgets(saved);
    let output = result.expect("a ceiling stop salvages a best answer");
    assert!(
        output.truncated,
        "the stop is the ceiling, not a natural completion: truncated={}, answer={:?}",
        output.truncated, output.answer
    );
    assert_eq!(
        counters.answering(),
        3,
        "exactly the planted turn ceiling of provider calls with tools \
         (answering={}, salvage={})",
        counters.answering(),
        counters.salvage()
    );
    assert_eq!(
        counters.salvage(),
        1,
        "the ceiling salvage made one final call without tools"
    );
}

// ===========================================================================
// The tool-call ceiling: SAYA_AGENT_MAX_TOOL_CALLS=2 must stop the loop when
// a turn's batch would cross it — after two executed calls and one refused
// batch, with the salvage call.
// ===========================================================================

#[tokio::test]
async fn env_tool_call_ceiling_stops_the_loop_at_the_planted_call_count() {
    let _env = ENV_LOCK.lock().await;
    let saved = plant_budgets(&[
        ("SAYA_AGENT_MAX_TURNS", None),
        ("SAYA_AGENT_MAX_TOOL_CALLS", Some("2")),
        ("SAYA_AGENT_MAX_CONTINUATIONS", None),
    ]);
    let (result, counters) =
        run_turn(256 * 1024, ProbeScript::ToolCalls { calls: 12 }, Vec::new()).await;
    restore_budgets(saved);
    let output = result.expect("a ceiling stop salvages a best answer");
    assert!(
        output.truncated,
        "the stop is the ceiling, not a natural completion: truncated={}, answer={:?}",
        output.truncated, output.answer
    );
    assert_eq!(
        counters.answering(),
        3,
        "two calls executed, the third turn's call was refused by the batch check \
         (answering={}, salvage={})",
        counters.answering(),
        counters.salvage()
    );
    assert_eq!(
        counters.salvage(),
        1,
        "the ceiling salvage made one final call without tools"
    );
}

// ===========================================================================
// The continuation ceiling: SAYA_AGENT_MAX_CONTINUATIONS=2 must re-instruct
// the model exactly twice on a deterministic truncation, then surface the
// truncation error.
// ===========================================================================

#[tokio::test]
async fn env_continuation_ceiling_bounds_the_truncation_retries() {
    let _env = ENV_LOCK.lock().await;
    let saved = plant_budgets(&[
        ("SAYA_AGENT_MAX_TURNS", None),
        ("SAYA_AGENT_MAX_TOOL_CALLS", None),
        ("SAYA_AGENT_MAX_CONTINUATIONS", Some("2")),
    ]);
    let (result, counters) = run_turn(256 * 1024, ProbeScript::AlwaysTruncated, Vec::new()).await;
    restore_budgets(saved);
    let error = result.expect_err("the final truncation must surface after the ceiling");
    assert!(
        error.to_string().contains("truncated"),
        "the truncation error surfaced, not something else: {error}"
    );
    assert_eq!(
        counters.answering(),
        3,
        "one initial call plus exactly the planted two continuations"
    );
    assert_eq!(counters.salvage(), 0, "a provider error salvages nothing");
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
    let _env = ENV_LOCK.lock().await;
    let saved = plant_budgets(&[
        ("SAYA_AGENT_MAX_TURNS", None),
        ("SAYA_AGENT_MAX_TOOL_CALLS", None),
        ("SAYA_AGENT_MAX_CONTINUATIONS", None),
    ]);
    let history = vec![
        ChatMessage::text(
            "user",
            format!("OLD-PAIR-SENTINEL-{}", "x".repeat(64 * 1024)),
        ),
        ChatMessage::text("assistant", "old answer"),
        ChatMessage::text("user", "NEW-PAIR-SENTINEL"),
        ChatMessage::text("assistant", "new answer"),
    ];
    let (result, counters) = run_turn(16 * 1024, ProbeScript::Answer, history).await;
    restore_budgets(saved);
    result.expect("the turn completes");
    assert_eq!(counters.answering(), 1, "one answering call");
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
