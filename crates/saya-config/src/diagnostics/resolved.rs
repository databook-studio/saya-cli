//! A display-safe view of the *effective* runtime settings — the type
//! `saya config show` prints.

use saya_types::CodingBudgetMode;
use serde::Serialize;
use std::collections::BTreeMap;

use super::{EndpointDiagnostics, redact_endpoint};
use crate::{
    ColorChoice, CompactionMode, JobRootMode, MemoryMode, OutputFormat, ResolvedConfig, ThemeChoice,
};

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
    /// Whether an `openrouter.ai` request carries OpenRouter's app-attribution
    /// headers; `false` is an explicit opt-out.
    pub app_attribution: bool,
    pub temperature: f32,
    pub timeout_seconds: u64,
    pub idle_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
    pub max_output_tokens: Option<u32>,
    pub reasoning_effort: Option<crate::ReasoningEffort>,
    pub retry_delays_ms: Vec<u64>,
    pub context_byte_budget: usize,
    pub context_window_tokens: Option<u64>,
    pub compaction_trigger_percent: u8,
    pub show_thinking: bool,
    pub max_rows: usize,
    pub read_only: bool,
    pub candidates: usize,
    /// The effective `[limits.session]` and `[limits.ask]` answering ceilings.
    /// `None` is the effective unlimited, stated by the key itself; `[limits.ask]`
    /// carries the session's value for every key it left unset.
    pub limits_session_requests: Option<u64>,
    pub limits_session_tool_calls: Option<u64>,
    pub limits_session_elapsed_seconds: Option<u64>,
    pub limits_session_known_tokens: Option<u64>,
    pub limits_ask_requests: Option<u64>,
    pub limits_ask_tool_calls: Option<u64>,
    pub limits_ask_elapsed_seconds: Option<u64>,
    pub limits_ask_known_tokens: Option<u64>,
    /// The effective `[limits.job]` mode and ceilings. `None` is the
    /// effective unlimited: `budget_mode = "uncapped"` states it, and
    /// `job_tokens` is `None` unless a layer declared a job-wide ceiling.
    pub limits_job_budget_mode: CodingBudgetMode,
    pub limits_job_logical_requests: Option<u64>,
    pub limits_job_transport_attempts: Option<u64>,
    pub limits_job_tool_calls: Option<u64>,
    pub limits_job_known_tokens: Option<u64>,
    pub limits_job_job_tokens: Option<u64>,
    pub limits_job_elapsed_seconds: Option<u64>,
    /// The progress guards: a count of zero is the effective "disabled".
    pub limits_job_no_progress_turns: u32,
    pub limits_job_repeat_limit: u32,
    pub limits_job_wall_clock_seconds: Option<u64>,
    /// The cost guard in US dollars; `None` is no guard.
    pub limits_job_max_cost_usd: Option<f64>,
    /// The two `[agents]` bounds, effective: the helper lane this job opens.
    pub agents_max_children: usize,
    pub agents_max_concurrent_model_requests: usize,
    pub job_program_dir: Option<String>,
    /// The effective root mode: `in_place` unless `[job] root_mode` says
    /// `managed`.
    pub job_root_mode: JobRootMode,
    /// The trusted selected LSP profile, if automatic post-edit diagnostics
    /// are enabled. Selection does not grant runner authority.
    pub job_automatic_lsp_profile: Option<String>,
    /// Safe effective profile summaries; argv bodies are never included.
    pub job_extensions: BTreeMap<String, serde_json::Value>,
    /// The effective helper agent types, keyed by the `subagent_type` a
    /// `helpers` call sends — the built-ins included, since these are the
    /// types the job's schema advertises.
    pub agents: BTreeMap<String, serde_json::Value>,
    /// The run's effective ceilings, from `[limits.run]`. `None` is the
    /// effective unlimited: nothing declared a ceiling for that dimension.
    pub limits_run_wall_clock_seconds: Option<u64>,
    pub limits_run_tokens_per_endpoint: BTreeMap<String, u64>,
    pub limits_run_turns: Option<u64>,
    pub limits_run_tool_calls: Option<u64>,
    pub limits_run_no_progress_turns: u32,
    pub limits_run_repeat_limit: u32,
    pub query_timeout_seconds: u64,
    /// The effective `[tools] allow` list, canonical and comma-separated, or
    /// `None` when no layer stated one — the default, which narrows nothing.
    pub tools_allow: Option<String>,
    /// Whether the effective configuration allows a session to enter
    /// `bypass`. A plain fact rather than an `Option`: the resolved answer is
    /// always known, and the protected table means a project layer's attempt
    /// never reaches it.
    pub permissions_allow_bypass: bool,
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
        let job = &self.limits.job;
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
            app_attribution: self.ai.app_attribution,
            temperature: self.ai.temperature,
            timeout_seconds: self.ai.timeout_seconds,
            idle_timeout_seconds: self.ai.idle_timeout_seconds,
            request_timeout_seconds: self.ai.request_timeout_seconds,
            max_output_tokens: self.ai.max_output_tokens,
            reasoning_effort: self.ai.reasoning_effort,
            retry_delays_ms: self.ai.retry_delays_ms.clone(),
            context_byte_budget: self.ai.context_byte_budget,
            context_window_tokens: self.ai.context_window_tokens,
            compaction_trigger_percent: self.ai.compaction_trigger_percent,
            show_thinking: self.ai.show_thinking,
            max_rows: self.max_rows,
            read_only: self.read_only,
            candidates: self.candidates,
            limits_session_requests: self.limits.session.requests.value(),
            limits_session_tool_calls: self.limits.session.tool_calls.value(),
            limits_session_elapsed_seconds: self.limits.session.elapsed_seconds.value(),
            limits_session_known_tokens: self.limits.session.known_tokens.value(),
            limits_ask_requests: self.limits.ask.requests.value(),
            limits_ask_tool_calls: self.limits.ask.tool_calls.value(),
            limits_ask_elapsed_seconds: self.limits.ask.elapsed_seconds.value(),
            limits_ask_known_tokens: self.limits.ask.known_tokens.value(),
            limits_job_budget_mode: job.budget_mode,
            limits_job_logical_requests: job.logical_requests.value(),
            limits_job_transport_attempts: job.transport_attempts.value(),
            limits_job_tool_calls: job.tool_calls.value(),
            limits_job_known_tokens: job.known_tokens.value(),
            limits_job_job_tokens: job.job_tokens.value(),
            limits_job_elapsed_seconds: job.elapsed_seconds.value(),
            limits_job_no_progress_turns: job.no_progress_turns,
            limits_job_repeat_limit: job.repeat_limit,
            limits_job_wall_clock_seconds: job.wall_clock_seconds.value(),
            limits_job_max_cost_usd: job
                .max_cost_micros
                .map(|micros| micros as f64 / 1_000_000.0),
            agents_max_children: self.agents.max_children,
            agents_max_concurrent_model_requests: self.agents.max_concurrent_model_requests,
            job_program_dir: self
                .coding
                .program_dir
                .as_ref()
                .map(|path| path.display().to_string()),
            job_root_mode: self.coding.root_mode,
            job_automatic_lsp_profile: self.coding.automatic_lsp_profile.clone(),
            job_extensions: self
                .coding
                .extensions
                .iter()
                .map(|(name, profile)| (name.clone(), profile.redacted_diagnostics()))
                .collect(),
            agents: self
                .agents
                .types
                .iter()
                .map(|(name, agent)| {
                    (
                        name.to_owned(),
                        serde_json::json!({
                            "role": agent.role.as_str(),
                            "allow_write": agent.allow_write,
                            "model": agent.model,
                        }),
                    )
                })
                .collect(),
            limits_run_wall_clock_seconds: self.limits.run.wall_clock_seconds.value(),
            limits_run_tokens_per_endpoint: self.limits.run.tokens_per_endpoint.clone(),
            limits_run_turns: self.limits.run.turns.value(),
            limits_run_tool_calls: self.limits.run.tool_calls.value(),
            limits_run_no_progress_turns: self.limits.run.no_progress_turns,
            limits_run_repeat_limit: self.limits.run.repeat_limit,
            query_timeout_seconds: self.query_timeout_seconds,
            tools_allow: self.tools.allow.as_ref().map(|allow| allow.joined()),
            permissions_allow_bypass: self.permissions.allow_bypass,
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
