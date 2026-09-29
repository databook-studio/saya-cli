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
    AgentMode, AgentOutput, ApprovalPolicy, CancellationToken, ChatMessage, ChatProvider,
    ChatRequest, ChatResponse, NoopEventSink,
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
    first_answering_messages: Mutex<Option<Vec<ChatMessage>>>,
}

impl Counters {
    fn first_messages(&self) -> Option<Vec<ChatMessage>> {
        self.first_answering_messages.lock().unwrap().clone()
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
        {
            let mut first = self.counters.first_answering_messages.lock().unwrap();
            if first.is_none() {
                *first = Some(request.messages.clone());
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
    let counters = Arc::new(Counters::default());
    let inputs = TurnInputs {
        ai: test_ai(context_byte_budget),
        provider: Box::new(AnswerProbe {
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
