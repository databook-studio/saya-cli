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
    ChatMessage, ChatProvider, ChatRequest, ChatResponse, NoopEventSink, ToolCall,
};
use saya_config::{
    AiProvider, BudgetLimit, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_connectors::{ConnectorOptions, SqliteConnector};
use saya_types::{
    Column, ConnectionError, Database, DatabaseProfile, ProfileIdentity, QueryRequest, QueryResult,
    Schema, SchemaTree, SqlDialect, Table,
};
use std::{
    collections::BTreeMap,
    num::NonZeroU64,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

include!("runtime_budget_tests/sqlite_registry.rs");
include!("runtime_budget_tests/runtime_config.rs");
include!("runtime_budget_tests/answer_probe.rs");
include!("runtime_budget_tests/request_measurement.rs");
// ===========================================================================
// The config context budget: `ai.context_byte_budget` is distinct from the
// built-in default (256 KiB, which would replay the oversized oldest pair)
// and from the harness's `max_rows` (100, which would replay nothing at all),
// so each wrong composition of `AgentLimits::context_byte_budget` is
// distinguishable from the right one by what the loop sent the provider.
// ===========================================================================

include!("runtime_budget_tests/context_budget.rs");

include!("runtime_budget_tests/tool_wallet.rs");
include!("runtime_budget_tests/reported_usage.rs");
