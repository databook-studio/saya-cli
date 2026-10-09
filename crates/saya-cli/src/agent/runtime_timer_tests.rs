//! Production-boundary coverage for the answering clock's extraction scope.

use super::{RecordingSink, assisted_memory, test_runtime};
use crate::{
    agent::runtime::{PromptOverrides, SinkPrompt, run_prompt_with_sink_and_outcomes},
    config::runtime::RuntimeConfig,
};
use saya_agent::{
    AgentEvent, AgentMode, ApprovalPolicy, BudgetDimension, CancellationToken, TokenUsage,
};
use saya_config::{
    AiProvider, BudgetLimit, ConnectionsFile, ResolvedAi, ResolvedInvestigationBudgets,
};
use saya_store::SqliteStateStore;
use saya_types::DatabaseProfile;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    num::NonZeroU64,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

include!("runtime_timer_tests/provider_source.rs");
include!("runtime_timer_tests/postgres_setup.rs");
include!("runtime_timer_tests/answering_deadlines.rs");

include!("runtime_timer_tests/pre_stopped.rs");
include!("runtime_timer_tests/supplied_scope.rs");
