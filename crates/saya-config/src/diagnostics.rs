use saya_types::CodingBudgetMode;
use serde::Serialize;

use crate::{
    ColorChoice, CompactionMode, ConfigFile, JobRootMode, MemoryMode, OutputFormat, ThemeChoice,
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
    /// The declared `[ai] app_attribution`, or `None` when the file did not
    /// state it — which is the effective `true`.
    pub app_attribution: Option<bool>,
    pub temperature: Option<f32>,
    pub read_only: Option<bool>,
    pub max_rows: Option<usize>,
    pub candidates: Option<usize>,
    /// The declared `[limits.session]` and `[limits.ask]` answering ceilings,
    /// verbatim: a dimension is an integer or the literal `"unlimited"`, and
    /// `None` means the file did not state it. Kept raw for the same reason as
    /// `[limits.run]`.
    pub limits_session_requests: Option<toml::Value>,
    pub limits_session_tool_calls: Option<toml::Value>,
    pub limits_session_elapsed_seconds: Option<toml::Value>,
    pub limits_session_known_tokens: Option<toml::Value>,
    pub limits_ask_requests: Option<toml::Value>,
    pub limits_ask_tool_calls: Option<toml::Value>,
    pub limits_ask_elapsed_seconds: Option<toml::Value>,
    pub limits_ask_known_tokens: Option<toml::Value>,
    /// The declared `[limits.job]` budget mode and ceilings, verbatim: a
    /// dimension is an integer, and `None` means the file did not state it.
    /// Kept raw rather than resolved for the same reason as `[limits.run]`.
    pub limits_job_budget_mode: Option<CodingBudgetMode>,
    pub limits_job_logical_requests: Option<toml::Value>,
    pub limits_job_transport_attempts: Option<toml::Value>,
    pub limits_job_tool_calls: Option<toml::Value>,
    pub limits_job_known_tokens: Option<toml::Value>,
    pub limits_job_job_tokens: Option<toml::Value>,
    pub limits_job_elapsed_seconds: Option<toml::Value>,
    pub limits_job_no_progress_turns: Option<toml::Value>,
    pub limits_job_repeat_limit: Option<toml::Value>,
    pub limits_job_wall_clock_seconds: Option<toml::Value>,
    pub limits_job_max_cost_usd: Option<toml::Value>,
    /// The two `[agents]` bounds and the registry, beside the coding settings:
    /// they are the helper lane a coding job opens, and a top-level section in
    /// their own right.
    pub agents_max_children: Option<usize>,
    pub agents_max_concurrent_model_requests: Option<usize>,
    pub job_program_dir: Option<String>,
    /// The declared `[job] root_mode`; `None` means the file left the default
    /// `in_place`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_root_mode: Option<JobRootMode>,
    /// Trusted optional selector only; profile arguments remain redacted in
    /// `job_extensions`.
    pub job_automatic_lsp_profile: Option<String>,
    /// Safe profile summaries; argv bodies are never included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_extensions: Option<BTreeMap<String, serde_json::Value>>,
    /// The declared `[agents]` entries, each carrying only the fields that
    /// entry stated — a type name, a role, a write flag, a model. `None` means
    /// the file declared no type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<BTreeMap<String, serde_json::Value>>,
    /// The declared `[limits.run]` ceilings, verbatim: a dimension is an
    /// integer or the literal `"unlimited"`, and `None` means the file did not
    /// state it. Kept raw rather than resolved because "declared unlimited"
    /// and "not declared" are different answers to the question this view
    /// asks, and both would otherwise read as `None`.
    pub limits_run_wall_clock_seconds: Option<toml::Value>,
    pub limits_run_tokens_per_endpoint: Option<BTreeMap<String, u64>>,
    pub limits_run_turns: Option<toml::Value>,
    pub limits_run_tool_calls: Option<toml::Value>,
    pub limits_run_no_progress_turns: Option<toml::Value>,
    pub limits_run_repeat_limit: Option<toml::Value>,
    pub query_timeout_seconds: Option<u64>,
    /// The declared `[tools] allow` list, verbatim: `None` means the file did
    /// not state it. Validation happens at resolve, so a declared list that
    /// does not name governed tools is reported there, not here.
    pub tools_allow: Option<Vec<String>>,
    /// The declared `[permissions] allow_bypass`, or `None` when the file did
    /// not state it. `None` is not `false`: the resolved default withholds
    /// the mode, but "nobody said" and "someone said no" are different
    /// answers to the question this view asks.
    pub permissions_allow_bypass: Option<bool>,
    pub output_format: Option<OutputFormat>,
    pub output_color: Option<ColorChoice>,
    pub ui_theme: Option<ThemeChoice>,
    pub compaction: Option<CompactionMode>,
    /// The declared `[ai] compaction_trigger_percent`, or `None` when the file
    /// did not state it — which is the effective 50.
    pub compaction_trigger_percent: Option<u8>,
    /// The declared `[ai] reasoning_effort`, or `None` when the file did not
    /// state it — which is not the same as any value: unset sends nothing.
    pub reasoning_effort: Option<crate::ReasoningEffort>,
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
            app_attribution: file.ai.app_attribution,
            temperature: file.ai.temperature,
            read_only: file.run.read_only,
            max_rows: file.run.max_rows,
            candidates: file.run.candidates,
            limits_session_requests: file.limits.session.requests.clone(),
            limits_session_tool_calls: file.limits.session.tool_calls.clone(),
            limits_session_elapsed_seconds: file.limits.session.elapsed_seconds.clone(),
            limits_session_known_tokens: file.limits.session.known_tokens.clone(),
            limits_ask_requests: file.limits.ask.requests.clone(),
            limits_ask_tool_calls: file.limits.ask.tool_calls.clone(),
            limits_ask_elapsed_seconds: file.limits.ask.elapsed_seconds.clone(),
            limits_ask_known_tokens: file.limits.ask.known_tokens.clone(),
            limits_job_budget_mode: file.limits.job.budget_mode,
            limits_job_logical_requests: file.limits.job.logical_requests.clone(),
            limits_job_transport_attempts: file.limits.job.transport_attempts.clone(),
            limits_job_tool_calls: file.limits.job.tool_calls.clone(),
            limits_job_known_tokens: file.limits.job.known_tokens.clone(),
            limits_job_job_tokens: file.limits.job.job_tokens.clone(),
            limits_job_elapsed_seconds: file.limits.job.elapsed_seconds.clone(),
            limits_job_no_progress_turns: file.limits.job.no_progress_turns.clone(),
            limits_job_repeat_limit: file.limits.job.repeat_limit.clone(),
            limits_job_wall_clock_seconds: file.limits.job.wall_clock_seconds.clone(),
            limits_job_max_cost_usd: file.limits.job.max_cost_usd.clone(),
            agents_max_children: file.agents.max_children,
            agents_max_concurrent_model_requests: file.agents.max_concurrent_model_requests,
            job_program_dir: file
                .job
                .program_dir
                .as_ref()
                .map(|path| path.display().to_string()),
            job_root_mode: file.job.root_mode,
            job_automatic_lsp_profile: file.job.automatic_lsp_profile.clone(),
            job_extensions: file.job.extensions.as_ref().map(|profiles| {
                profiles
                    .iter()
                    .map(|(name, profile)| (name.clone(), profile.redacted_diagnostics()))
                    .collect()
            }),
            agents: (!file.agents.types.is_empty()).then(|| {
                file.agents
                    .types
                    .iter()
                    .map(|(name, entry)| (name.clone(), declared_agent(entry)))
                    .collect()
            }),
            limits_run_wall_clock_seconds: file.limits.run.wall_clock_seconds.clone(),
            limits_run_tokens_per_endpoint: file.limits.run.tokens_per_endpoint.clone(),
            limits_run_turns: file.limits.run.turns.clone(),
            limits_run_tool_calls: file.limits.run.tool_calls.clone(),
            limits_run_no_progress_turns: file.limits.run.no_progress_turns.clone(),
            limits_run_repeat_limit: file.limits.run.repeat_limit.clone(),
            query_timeout_seconds: file.run.query_timeout_seconds,
            tools_allow: file.tools.allow.clone(),
            permissions_allow_bypass: file.permissions.allow_bypass,
            output_format: file.output.format,
            output_color: file.output.color,
            ui_theme: file.ui.theme,
            compaction: file.ai.compaction,
            compaction_trigger_percent: file.ai.compaction_trigger_percent,
            reasoning_effort: file.ai.reasoning_effort,
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

/// What one declared `[agents]` entry states, field by field: an absent
/// field is left out rather than shown as `null`, because "the entry did not
/// state it" is the question this view answers.
fn declared_agent(entry: &crate::AgentFile) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    if let Some(role) = entry.role {
        object.insert("role".to_owned(), role.as_str().into());
    }
    if let Some(allow_write) = entry.allow_write {
        object.insert("allow_write".to_owned(), allow_write.into());
    }
    if let Some(model) = entry.model.as_deref() {
        object.insert("model".to_owned(), model.into());
    }
    serde_json::Value::Object(object)
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
