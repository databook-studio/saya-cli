//! A display-safe view of the *effective* runtime settings — the type
//! `saya config show` prints.

use serde::Serialize;
use std::collections::BTreeMap;

use super::EndpointDiagnostics;
use crate::{ColorChoice, CompactionMode, MemoryMode, OutputFormat, ThemeChoice};

/// A display-safe view of the *effective* runtime settings, with no resolved
/// secrets. This is what `saya config show` prints.
///
/// Fields are concrete rather than optional: every one has a value once the
/// layers and defaults have been applied. Use [`crate::RedactedDiagnostics`] instead
/// when the question is which of them a config file actually declared.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedDiagnostics {
    pub profile_name: Option<String>,
    pub profile_dialect: Option<String>,
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key_reference: Option<String>,
    pub allow_data_sharing: bool,
    pub temperature: f32,
    pub timeout_seconds: u64,
    pub idle_timeout_seconds: u64,
    pub max_output_tokens: u32,
    pub retry_delays_ms: Vec<u64>,
    pub context_byte_budget: usize,
    pub context_window_tokens: Option<u64>,
    pub show_thinking: bool,
    pub max_rows: usize,
    pub read_only: bool,
    pub max_iterations: usize,
    pub candidates: usize,
    pub jobs_wall_clock_seconds: Option<u64>,
    pub jobs_tokens_per_endpoint: BTreeMap<String, u64>,
    pub jobs_turns: Option<u64>,
    pub jobs_tool_calls: Option<u64>,
    pub jobs_runner_allow: Vec<String>,
    pub jobs_runner_program_dir: Option<String>,
    pub jobs_runner_timeout_seconds: u64,
    pub query_timeout_seconds: u64,
    pub output_format: OutputFormat,
    pub output_color: ColorChoice,
    pub ui_theme: ThemeChoice,
    pub compaction: CompactionMode,
    pub memory_mode: MemoryMode,
    pub memory_max_contracts: u32,
    pub memory_max_claims_per_contract: u32,
    pub memory_max_context_bytes: u32,
    /// The resolved endpoint pool, keyed by name — always contains
    /// `orchestrator`, the plain `[ai]` block fallback. Secrets stay
    /// references: `api_key_reference` is the redacted label, never a value.
    pub endpoints: BTreeMap<String, EndpointDiagnostics>,
}
