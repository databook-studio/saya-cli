use saya_types::CheckDiagnosticFormat;
use serde::Serialize;

use crate::{
    ColorChoice, CompactionMode, ConfigFile, MemoryMode, OutputFormat, ThemeChoice,
    coding::ResolvedCheckProfile, model::CheckProfileFile,
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
    pub coding_enabled: Option<bool>,
    pub coding_max_children: Option<usize>,
    pub coding_max_concurrent_model_requests: Option<usize>,
    pub coding_max_concurrent_commands: Option<usize>,
    pub coding_max_logical_model_requests: Option<usize>,
    pub coding_max_physical_transport_attempts: Option<usize>,
    pub coding_max_tool_calls: Option<usize>,
    pub coding_max_known_tokens: Option<u64>,
    pub coding_max_elapsed_seconds: Option<u64>,
    /// Safe profile summaries; argv bodies are never included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coding_extensions: Option<BTreeMap<String, serde_json::Value>>,
    pub coding_check_profiles: Option<BTreeMap<String, CheckProfileDiagnostics>>,
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
            coding_enabled: file.coding.enabled,
            coding_max_children: file.coding.max_children,
            coding_max_concurrent_model_requests: file.coding.max_concurrent_model_requests,
            coding_max_concurrent_commands: file.coding.max_concurrent_commands,
            coding_max_logical_model_requests: file.coding.max_logical_model_requests,
            coding_max_physical_transport_attempts: file.coding.max_physical_transport_attempts,
            coding_max_tool_calls: file.coding.max_tool_calls,
            coding_max_known_tokens: file.coding.max_known_tokens,
            coding_max_elapsed_seconds: file.coding.max_elapsed_seconds,
            coding_extensions: file.coding.extensions.as_ref().map(|profiles| {
                profiles
                    .iter()
                    .map(|(name, profile)| (name.clone(), profile.redacted_diagnostics()))
                    .collect()
            }),
            coding_check_profiles: file.coding.check_profiles.as_ref().map(|profiles| {
                profiles
                    .iter()
                    .map(|(name, profile)| {
                        (name.clone(), CheckProfileDiagnostics::from_file(profile))
                    })
                    .collect()
            }),
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

/// Display-safe identity and bounds for a configured coding check.
#[derive(Debug, Clone, Serialize)]
pub struct CheckProfileDiagnostics {
    pub command_id: String,
    pub program: String,
    pub argv_count: usize,
    pub timeout_seconds: u64,
    pub toolchain_reference: Option<String>,
    pub environment_profile_reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_format: Option<CheckDiagnosticFormat>,
}

impl CheckProfileDiagnostics {
    fn from_file(profile: &CheckProfileFile) -> Self {
        Self {
            command_id: profile.command_id.clone(),
            program: profile.program.clone(),
            argv_count: profile.argv.len(),
            timeout_seconds: profile.timeout_seconds,
            toolchain_reference: profile.toolchain.clone(),
            environment_profile_reference: profile.environment_profile.clone(),
            diagnostic_format: profile.diagnostic_format,
        }
    }

    pub(crate) fn from_resolved(profile: &ResolvedCheckProfile) -> Self {
        Self {
            command_id: profile.command_id().to_owned(),
            program: profile.program().to_owned(),
            argv_count: profile.argv_count(),
            timeout_seconds: profile.timeout_seconds(),
            toolchain_reference: profile.toolchain_reference().map(str::to_owned),
            environment_profile_reference: profile.environment_profile_reference().to_owned(),
            diagnostic_format: profile.diagnostic_format(),
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

mod endpoint;
mod resolved;

pub use endpoint::EndpointDiagnostics;
pub use resolved::ResolvedDiagnostics;
