//! The headless automatic-compaction adapter: the decision's numerator is the
//! LAST answering request's input tokens (never the summed run usage, never a
//! failed attempt's report, never an invented zero), every suppression gate
//! the TUI reads also gates the plain REPL, and a pending clarification or a
//! stop never authorises automatic work. The end-to-end firing through the
//! real REPL loop is the next packet (C4-3b); these tests pin the decision
//! and its gating.

use saya_agent::{AgentEvent, AgentOutput, TokenUsage, UsageCall};
use std::path::PathBuf;

use saya_config::{
    AiProvider, ColorChoice, CompactionMode, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};

/// The explicit runtime the decision reads (the `session_headless_receipt_tests`
/// pattern, so this module mounts at the interactive level like its neighbours):
/// auto mode, no profile, the stated window — a struct literal so no config,
/// env, or connection file is touched.
fn runtime_with_window(window: Option<u64>) -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
            coding: Default::default(),
            instructions: Default::default(),
            profile_name: None,
            profile: None,
            investigation_budgets: Default::default(),
            ai: ResolvedAi {
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
                context_byte_budget: 256 * 1024,
                context_window_tokens: window,
                show_thinking: false,
                compaction: CompactionMode::Auto,
                retry_delays_ms: vec![250, 500, 1000],
            },
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            candidates: 1,
            jobs: ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: Default::default(),
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
            provenance: Default::default(),
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-c43a-decision"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

/// A settled run output with the given events, summed usage, and salvage flag.
fn output(events: Vec<AgentEvent>, usage: TokenUsage, truncated: bool) -> AgentOutput {
    AgentOutput {
        answer: "the answer".into(),
        events,
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage,
        learning_usage: None,
        truncated,
        answer_sql: None,
    }
}

/// One answering round's usage report, as `receive` emits it mid-turn.
fn answering_report(input: u64) -> AgentEvent {
    AgentEvent::usage(UsageCall::Answer, TokenUsage::new(input, 20))
}

#[path = "headless_tests/decision_table.rs"]
mod decision_table;

#[path = "headless_tests/settle_gate.rs"]
mod settle_gate;
