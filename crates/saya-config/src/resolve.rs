use saya_types::DatabaseProfile;

use crate::{
    AiProvider, ColorChoice, ConfigError, ConfigFile, OutputFormat, ResolutionInput,
    layers::{apply_cli, apply_env, merge, revert_untrusted, snapshot_protected},
    memory::ResolvedMemory,
    profile_env::overlay_database_environment,
};

const DEFAULT_MODEL: &str = "qwen2.5-coder:14b";

/// Default conversation byte budget: the 256 KiB the agent loop used before
/// this setting existed (Invariant 1 — a user with no setting changes nothing).
const DEFAULT_CONTEXT_BYTE_BUDGET: usize = 256 * 1024;

/// Smallest accepted `[ai] context_byte_budget`. Below this the budget is too
/// small to hold a system prompt and a single turn, so the loop would trim away
/// useful context on every turn — a budget of 0 trims the conversation to
/// nothing. Matched against the existing `[memory]` range-check style rather
/// than a silent clamp (Invariant 3). No upper bound: a user with a large
/// context window may raise it freely, which is the point of making it settable.
const MIN_CONTEXT_BYTE_BUDGET: usize = 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub profile_name: Option<String>,
    pub profile: Option<DatabaseProfile>,
    pub ai: ResolvedAi,
    pub max_rows: usize,
    pub read_only: bool,
    pub max_iterations: usize,
    pub query_timeout_seconds: u64,
    pub output_format: OutputFormat,
    pub output_color: ColorChoice,
    pub memory: ResolvedMemory,
    /// Security-critical setting names (`ai.base_url`, `run.read_only`, …)
    /// that the project layer tried to override and were ignored. Empty when
    /// the project layer is trusted or set none of them.
    pub ignored_project_overrides: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAi {
    pub provider: AiProvider,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<saya_types::SecretRef>,
    pub allow_data_sharing: bool,
    /// Sampling temperature for the LLM (lower = more concise/deterministic).
    pub temperature: f32,
    /// Budget for request establishment / non-streaming responses.
    pub timeout_seconds: u64,
    /// Maximum silence between stream chunks before the provider is stalled.
    pub idle_timeout_seconds: u64,
    /// Per-response output-token ceiling requested from the provider.
    pub max_output_tokens: u32,
    /// Ceiling on the approximate byte size of the conversation the agent loop
    /// assembles. The loop trims under it instead of aborting, so a user on a
    /// model with a large context window can raise it to keep more history.
    pub context_byte_budget: usize,
    /// Show the model's chain-of-thought in the transcript. Off by default;
    /// display only — reasoning is never persisted regardless of this setting.
    pub show_thinking: bool,
}

pub fn resolve(input: ResolutionInput) -> Result<ResolvedConfig, ConfigError> {
    let mut file = ConfigFile::default();
    if let Some(user) = input.user.as_ref() {
        merge(&mut file, user);
    }
    let protected = snapshot_protected(&file);
    if let Some(project) = input.project.as_ref() {
        merge(&mut file, project);
    }
    let ignored_project_overrides = if input.cli.trust_project_config {
        Vec::new()
    } else {
        revert_untrusted(&mut file, &protected)
    };
    apply_env(&mut file, &input.env_file)?;
    apply_env(&mut file, &input.process_env)?;
    apply_cli(&mut file, &input.cli);
    let selected = input
        .cli
        .profile
        .or_else(|| input.process_env.get("SAYA_PROFILE").cloned())
        .or(file.default_profile.clone())
        .or_else(|| {
            (input.connections.profiles.len() == 1)
                .then(|| input.connections.profiles.keys().next().cloned())
                .flatten()
        });
    if selected.is_none() && input.connections.profiles.len() > 1 {
        return Err(ConfigError::MissingProfile);
    }
    let environment_profile = input.env_file.contains_key("SAYA_DB_TYPE")
        || input.process_env.contains_key("SAYA_DB_TYPE");
    let profile = selected
        .as_ref()
        .map(|name| match input.connections.profiles.get(name).cloned() {
            Some(profile) => Ok(Some(profile)),
            None if environment_profile => Ok(None),
            None => Err(ConfigError::UnknownProfile(name.clone())),
        })
        .transpose()?
        .flatten();
    let profile = overlay_database_environment(profile, &input.env_file, &input.process_env)?;
    let memory = crate::memory::resolve(&file.memory)?;
    let context_byte_budget = file
        .ai
        .context_byte_budget
        .unwrap_or(DEFAULT_CONTEXT_BYTE_BUDGET);
    require_context_byte_budget(context_byte_budget)?;
    Ok(ResolvedConfig {
        profile_name: selected,
        profile,
        ai: ResolvedAi {
            provider: file.ai.provider.unwrap_or(AiProvider::Ollama),
            model: file.ai.model.unwrap_or_else(|| DEFAULT_MODEL.into()),
            base_url: file.ai.base_url,
            api_key: file.ai.api_key,
            allow_data_sharing: file.ai.allow_data_sharing.unwrap_or(false),
            temperature: file.ai.temperature.unwrap_or(0.1),
            timeout_seconds: file.ai.timeout_seconds.unwrap_or(60),
            idle_timeout_seconds: file.ai.idle_timeout_seconds.unwrap_or(90),
            max_output_tokens: file.ai.max_output_tokens.unwrap_or(4096),
            context_byte_budget,
            show_thinking: file.ai.show_thinking.unwrap_or(false),
        },
        max_rows: file.run.max_rows.unwrap_or(1000),
        read_only: file.run.read_only.unwrap_or(true),
        max_iterations: file.run.max_iterations.unwrap_or(12),
        query_timeout_seconds: file.run.query_timeout_seconds.unwrap_or(60),
        output_format: file.output.format.unwrap_or(OutputFormat::Text),
        output_color: file.output.color.unwrap_or(ColorChoice::Auto),
        memory,
        ignored_project_overrides,
    })
}

/// Rejects an `[ai] context_byte_budget` below the floor with a typed error
/// (Invariant 3), matching the `[memory]` range-check style. The upper end is
/// unbounded: a user may raise the budget to fit a larger context window, which
/// is the reason the setting exists, so no ceiling is enforced here.
fn require_context_byte_budget(value: usize) -> Result<(), ConfigError> {
    if value >= MIN_CONTEXT_BYTE_BUDGET {
        Ok(())
    } else {
        Err(ConfigError::SettingBelowMinimum {
            field: "context_byte_budget",
            value,
            min: MIN_CONTEXT_BYTE_BUDGET,
        })
    }
}
