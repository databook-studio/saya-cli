//! A display-safe view of the *effective* runtime settings — the type
//! `saya config show` prints.

use serde::Serialize;
use std::collections::BTreeMap;

use super::{CheckProfileDiagnostics, EndpointDiagnostics, redact_endpoint};
use crate::{ColorChoice, CompactionMode, MemoryMode, OutputFormat, ResolvedConfig, ThemeChoice};

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
    pub coding_enabled: bool,
    pub coding_max_children: usize,
    pub coding_max_concurrent_model_requests: usize,
    pub coding_max_concurrent_commands: usize,
    pub coding_max_logical_model_requests: usize,
    pub coding_max_physical_transport_attempts: usize,
    pub coding_max_tool_calls: usize,
    pub coding_max_known_tokens: u64,
    pub coding_max_elapsed_seconds: u64,
    /// Safe effective profile summaries; argv bodies are never included.
    pub coding_extensions: BTreeMap<String, serde_json::Value>,
    pub coding_check_profiles: BTreeMap<String, CheckProfileDiagnostics>,
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

impl ResolvedConfig {
    pub fn redacted_diagnostics(&self) -> ResolvedDiagnostics {
        ResolvedDiagnostics {
            profile_name: self.profile_name.clone(),
            profile_dialect: self
                .profile
                .as_ref()
                .map(|value| value.dialect().as_str().into()),
            provider: self.ai.provider.as_str().into(),
            model: self.ai.model.clone(),
            base_url: self.ai.base_url.as_deref().map(redact_endpoint),
            api_key_reference: self.ai.api_key.as_ref().map(|value| value.redacted_label()),
            allow_data_sharing: self.ai.allow_data_sharing,
            temperature: self.ai.temperature,
            timeout_seconds: self.ai.timeout_seconds,
            idle_timeout_seconds: self.ai.idle_timeout_seconds,
            max_output_tokens: self.ai.max_output_tokens,
            retry_delays_ms: self.ai.retry_delays_ms.clone(),
            context_byte_budget: self.ai.context_byte_budget,
            context_window_tokens: self.ai.context_window_tokens,
            show_thinking: self.ai.show_thinking,
            max_rows: self.max_rows,
            read_only: self.read_only,
            max_iterations: self.max_iterations,
            candidates: self.candidates,
            coding_enabled: self.coding.enabled,
            coding_max_children: self.coding.max_children,
            coding_max_concurrent_model_requests: self.coding.max_concurrent_model_requests,
            coding_max_concurrent_commands: self.coding.max_concurrent_commands,
            coding_max_logical_model_requests: self.coding.max_logical_model_requests,
            coding_max_physical_transport_attempts: self.coding.max_physical_transport_attempts,
            coding_max_tool_calls: self.coding.max_tool_calls,
            coding_max_known_tokens: self.coding.max_known_tokens,
            coding_max_elapsed_seconds: self.coding.max_elapsed_seconds,
            coding_extensions: self
                .coding
                .extensions
                .iter()
                .map(|(name, profile)| (name.clone(), profile.redacted_diagnostics()))
                .collect(),
            coding_check_profiles: self
                .coding
                .check_profiles
                .iter()
                .map(|(name, profile)| {
                    (
                        name.clone(),
                        CheckProfileDiagnostics::from_resolved(profile),
                    )
                })
                .collect(),
            jobs_wall_clock_seconds: self.jobs.wall_clock_seconds,
            jobs_tokens_per_endpoint: self.jobs.tokens_per_endpoint.clone(),
            jobs_turns: self.jobs.turns,
            jobs_tool_calls: self.jobs.tool_calls,
            jobs_runner_allow: self.jobs.runner.allow.clone(),
            jobs_runner_program_dir: self
                .jobs
                .runner
                .program_dir
                .as_ref()
                .map(|path| path.display().to_string()),
            jobs_runner_timeout_seconds: self.jobs.runner.timeout_seconds,
            query_timeout_seconds: self.query_timeout_seconds,
            output_format: self.output_format,
            output_color: self.output_color,
            ui_theme: self.ui_theme,
            compaction: self.ai.compaction,
            memory_mode: self.memory.mode,
            memory_max_contracts: self.memory.max_contracts,
            memory_max_claims_per_contract: self.memory.max_claims_per_contract,
            memory_max_context_bytes: self.memory.max_context_bytes,
            endpoints: self
                .endpoints
                .iter()
                .map(|(name, endpoint)| {
                    (
                        name.clone(),
                        EndpointDiagnostics {
                            name: endpoint.name.clone(),
                            provider: Some(endpoint.provider.as_str().into()),
                            model: Some(endpoint.model.clone()),
                            base_url: endpoint.base_url.as_deref().map(redact_endpoint),
                            api_key_reference: endpoint
                                .api_key
                                .as_ref()
                                .map(|value| value.redacted_label()),
                        },
                    )
                })
                .collect(),
        }
    }
}
