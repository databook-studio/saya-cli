use saya_types::DatabaseProfile;

use crate::{
    AiProvider, ColorChoice, ConfigError, ConfigFile, OutputFormat, ResolutionInput,
    layers::{apply_cli, apply_env, merge},
    profile_env::overlay_database_environment,
};

const DEFAULT_MODEL: &str = "qwen2.5-coder:14b";

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
}

pub fn resolve(input: ResolutionInput) -> Result<ResolvedConfig, ConfigError> {
    let mut file = ConfigFile::default();
    if let Some(user) = input.user.as_ref() {
        merge(&mut file, user);
    }
    if let Some(project) = input.project.as_ref() {
        merge(&mut file, project);
    }
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
        },
        max_rows: file.run.max_rows.unwrap_or(1000),
        read_only: file.run.read_only.unwrap_or(true),
        max_iterations: file.run.max_iterations.unwrap_or(12),
        query_timeout_seconds: file.run.query_timeout_seconds.unwrap_or(60),
        output_format: file.output.format.unwrap_or(OutputFormat::Text),
        output_color: file.output.color.unwrap_or(ColorChoice::Auto),
    })
}
