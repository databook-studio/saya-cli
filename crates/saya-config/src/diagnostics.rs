use serde::Serialize;

use crate::{
    ColorChoice, CompactionMode, ConfigFile, MemoryMode, OutputFormat, ResolvedConfig, ThemeChoice,
};
use std::collections::BTreeMap;

/// A display-safe view of what a config *file* declares; references are
/// retained, values are not.
///
/// Every field is an `Option`, and that is the point: `None` means the file did
/// not set it. This answers "what did I configure?", which
/// [`ResolvedDiagnostics`] cannot — the resolved view reports the effective
/// value after every layer and default is applied, so a setting left unset and
/// a setting explicitly set to the default look identical there.
///
/// **No binary in this workspace consumes this type, and that is deliberate.**
/// `saya config show` prints the resolved view, because "what is in effect" is
/// the question a user debugging a connection is asking. This one exists for
/// callers of `saya-config` as a library — the crate is published, and reading
/// a config file to see what it declares is a coherent thing to want without
/// running the CLI. It is exercised by the doctest below and by this crate's
/// own tests, not by dead-code accident.
///
/// ```
/// use saya_config::ConfigFile;
///
/// // `max_rows` is absent, so the file view reports it as unset rather than
/// // as the default the resolved view would show.
/// let file = ConfigFile::from_toml("[ai]\nmodel = \"claude-opus-4\"\n").unwrap();
/// let declared = file.redacted_diagnostics();
///
/// assert_eq!(declared.model.as_deref(), Some("claude-opus-4"));
/// assert_eq!(declared.max_rows, None);
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct RedactedDiagnostics {
    pub default_profile: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key_reference: Option<String>,
    pub allow_data_sharing: Option<bool>,
    pub temperature: Option<f32>,
    pub read_only: Option<bool>,
    pub max_rows: Option<usize>,
    pub max_iterations: Option<usize>,
    pub candidates: Option<usize>,
    pub jobs_wall_clock_seconds: Option<u64>,
    pub jobs_tokens_per_endpoint: Option<BTreeMap<String, u64>>,
    pub jobs_turns: Option<u64>,
    pub jobs_tool_calls: Option<u64>,
    pub jobs_runner_allow: Option<Vec<String>>,
    pub jobs_runner_program_dir: Option<String>,
    pub jobs_runner_timeout_seconds: Option<u64>,
    pub query_timeout_seconds: Option<u64>,
    pub output_format: Option<OutputFormat>,
    pub output_color: Option<ColorChoice>,
    pub ui_theme: Option<ThemeChoice>,
    pub compaction: Option<CompactionMode>,
    pub memory_mode: Option<MemoryMode>,
    pub memory_max_contracts: Option<u32>,
    pub memory_max_claims_per_contract: Option<u32>,
    pub memory_max_context_bytes: Option<u32>,
    /// Declared `[[ai.endpoints]]`, keyed by name. `None` means the file did
    /// not declare that field. Secrets stay references.
    pub endpoints: BTreeMap<String, EndpointDiagnostics>,
}

/// A display-safe view of the *effective* runtime settings, with no resolved
/// secrets. This is what `saya config show` prints.
///
/// Fields are concrete rather than optional: every one has a value once the
/// layers and defaults have been applied. Use [`RedactedDiagnostics`] instead
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

/// One AI endpoint mirrored for display. `Option` means "not declared" in
/// the file view and "resolved to none" in the resolved view; either way a
/// secret appears only as its redacted reference label, never as a value —
/// the same discipline as `api_key_reference` on the parent views.
#[derive(Debug, Clone, Serialize)]
pub struct EndpointDiagnostics {
    pub name: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key_reference: Option<String>,
}

impl RedactedDiagnostics {
    pub(crate) fn from_file(file: &ConfigFile) -> Self {
        Self {
            default_profile: file.default_profile.clone(),
            provider: file.ai.provider.map(|value| value.as_str().into()),
            model: file.ai.model.clone(),
            base_url: file.ai.base_url.as_deref().map(redact_endpoint),
            api_key_reference: file.ai.api_key.as_ref().map(|value| value.redacted_label()),
            allow_data_sharing: file.ai.allow_data_sharing,
            temperature: file.ai.temperature,
            read_only: file.run.read_only,
            max_rows: file.run.max_rows,
            max_iterations: file.run.max_iterations,
            candidates: file.run.candidates,
            jobs_wall_clock_seconds: file.jobs.wall_clock_seconds,
            jobs_tokens_per_endpoint: file.jobs.tokens_per_endpoint.clone(),
            jobs_turns: file.jobs.turns,
            jobs_tool_calls: file.jobs.tool_calls,
            jobs_runner_allow: file.jobs.runner.as_ref().and_then(|r| r.allow.clone()),
            jobs_runner_program_dir: file
                .jobs
                .runner
                .as_ref()
                .and_then(|r| r.program_dir.as_ref())
                .map(|path| path.display().to_string()),
            jobs_runner_timeout_seconds: file.jobs.runner.as_ref().and_then(|r| r.timeout_seconds),
            query_timeout_seconds: file.run.query_timeout_seconds,
            output_format: file.output.format,
            output_color: file.output.color,
            ui_theme: file.ui.theme,
            compaction: file.ai.compaction,
            memory_mode: file.memory.mode,
            memory_max_contracts: file.memory.max_contracts,
            memory_max_claims_per_contract: file.memory.max_claims_per_contract,
            memory_max_context_bytes: file.memory.max_context_bytes,
            endpoints: file
                .ai
                .endpoints
                .iter()
                .map(|endpoint| {
                    (
                        endpoint.name.clone(),
                        EndpointDiagnostics {
                            name: endpoint.name.clone(),
                            provider: endpoint.provider.map(|value| value.as_str().into()),
                            model: endpoint.model.clone(),
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

fn redact_endpoint(value: &str) -> String {
    let mut output = redact_userinfo(value);
    if let Some(index) = output.find('?') {
        output.truncate(index + 1);
        output.push_str("[redacted]");
    }
    output
}

fn redact_userinfo(value: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    while let Some(offset) = value[cursor..].find("://") {
        let scheme = cursor + offset;
        let auth_start = scheme + 3;
        let rest = &value[auth_start..];
        let Some(at_offset) = rest.find('@') else {
            break;
        };
        let boundary = rest
            .find(|character: char| "/?# \t\r\n".contains(character))
            .unwrap_or(rest.len());
        if at_offset >= boundary {
            cursor = auth_start;
            continue;
        }
        let at = auth_start + at_offset;
        output.push_str(&value[cursor..auth_start]);
        cursor = at + 1;
    }
    output.push_str(&value[cursor..]);
    output
}
