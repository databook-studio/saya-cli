//! The session collector's Compaction phase, recorded by the shared
//! compaction operation: one sample per `/compact` invocation — Succeeded
//! when a summary validated and applied, Failed when the provider or
//! validation refused, Skipped for a no-work result — with
//! `CompactionStarted` counted only when the summariser call actually
//! starts. A collector-less run is otherwise identical.

use super::run_with_trigger;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::state::SessionState;
use crate::interactive::session::universe::SessionUniverse;
use crate::local_metrics::{
    LocalMetrics, LocalPhaseSnapshot, PhaseCell, count_index, outcome_index, phase_index,
};
use saya_agent::{AgentMetricCount, AgentMetricOutcome, AgentMetricPhase};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_store::StoredContinuation;
use saya_types::NarrativeSource;
use std::{
    io::{Read, Write as _},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
include!("compact_task_metrics_tests/support.rs");
include!("compact_task_metrics_tests/outcomes.rs");

include!("compact_task_metrics_tests/continuation.rs");
