use std::collections::BTreeMap;

use saya_types::DatabaseProfile;

use crate::{AiProvider, CliOverrides, ConfigError, ConfigFile, OutputFormat, ResolutionInput};

const DEFAULT_MODEL: &str = "qwen2.5-coder:14b";

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub profile_name: Option<String>,
    pub profile: Option<DatabaseProfile>,
    pub ai: ResolvedAi,
    pub max_rows: usize,
    pub read_only: bool,
    pub output_format: OutputFormat,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAi {
    pub provider: AiProvider,
    pub model: String,
    pub allow_data_sharing: bool,
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
    let profile = selected
        .as_ref()
        .map(|name| {
            input
                .connections
                .profiles
                .get(name)
                .cloned()
                .ok_or_else(|| ConfigError::UnknownProfile(name.clone()))
        })
        .transpose()?;
    Ok(ResolvedConfig {
        profile_name: selected,
        profile,
        ai: ResolvedAi {
            provider: file.ai.provider.unwrap_or(AiProvider::Ollama),
            model: file.ai.model.unwrap_or_else(|| DEFAULT_MODEL.into()),
            allow_data_sharing: file.ai.allow_data_sharing.unwrap_or(false),
        },
        max_rows: file.run.max_rows.unwrap_or(1000),
        read_only: file.run.read_only.unwrap_or(true),
        output_format: file.output.format.unwrap_or(OutputFormat::Text),
    })
}

fn merge(base: &mut ConfigFile, layer: &ConfigFile) {
    macro_rules! apply { ($($path:ident).+) => { if layer.$($path).+.is_some() { base.$($path).+ = layer.$($path).+.clone(); } }; }
    apply!(default_profile);
    apply!(ai.provider);
    apply!(ai.model);
    apply!(ai.base_url);
    apply!(ai.allow_data_sharing);
    apply!(ai.api_key);
    apply!(run.read_only);
    apply!(run.max_rows);
    apply!(run.max_iterations);
    apply!(run.query_timeout_seconds);
    apply!(output.format);
    apply!(output.color);
}

fn apply_env(file: &mut ConfigFile, env: &BTreeMap<String, String>) -> Result<(), ConfigError> {
    if let Some(value) = env.get("SAYA_AI_MODEL") {
        file.ai.model = Some(value.clone());
    }
    if let Some(value) = env.get("SAYA_MAX_ROWS") {
        file.run.max_rows = Some(value.parse().map_err(|_| ConfigError::InvalidEnvironment {
            name: "SAYA_MAX_ROWS".into(),
            reason: "expected an unsigned integer".into(),
        })?);
    }
    if let Some(value) = env.get("SAYA_ALLOW_DATA_SHARING") {
        file.ai.allow_data_sharing =
            Some(value.parse().map_err(|_| ConfigError::InvalidEnvironment {
                name: "SAYA_ALLOW_DATA_SHARING".into(),
                reason: "expected true or false".into(),
            })?);
    }
    Ok(())
}

fn apply_cli(file: &mut ConfigFile, cli: &CliOverrides) {
    if cli.provider.is_some() {
        file.ai.provider = cli.provider;
    }
    if cli.model.is_some() {
        file.ai.model = cli.model.clone();
    }
    if cli.allow_data_sharing.is_some() {
        file.ai.allow_data_sharing = cli.allow_data_sharing;
    }
    if cli.max_rows.is_some() {
        file.run.max_rows = cli.max_rows;
    }
}
