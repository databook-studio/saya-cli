use crate::{
    cli::GlobalOptions,
    config_sources::{self, Paths},
};
use saya_config::{CliOverrides, ConnectionsFile, ResolutionInput, resolve};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("{0}")]
    Config(#[from] saya_config::ConfigError),
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("explicit path does not exist: {0}")]
    Missing(PathBuf),
    #[error("invalid approval mode: {0}")]
    Approval(String),
}

#[derive(Clone)]
pub struct RuntimeConfig {
    pub resolved: saya_config::ResolvedConfig,
    pub connections: ConnectionsFile,
    pub config_path: Option<PathBuf>,
    pub connections_path: Option<PathBuf>,
    pub(crate) secret_values: BTreeMap<String, String>,
}

pub fn load(options: &GlobalOptions, cwd: &Path) -> Result<RuntimeConfig, RuntimeError> {
    load_with_sources(
        options,
        cwd,
        &config_sources::user_config_dir(),
        config_sources::process_env(),
    )
}

pub fn load_with_sources(
    options: &GlobalOptions,
    cwd: &Path,
    user_dir: &Path,
    process: BTreeMap<String, String>,
) -> Result<RuntimeConfig, RuntimeError> {
    let paths = Paths::discover(cwd, user_dir);
    let user = config_sources::read_config(&paths.user_config)?;
    let selected_config = options
        .config
        .as_ref()
        .or_else(|| paths.project_config.as_ref().filter(|path| path.exists()))
        .or_else(|| paths.user_config.as_ref().filter(|path| path.exists()));
    let project = match options.config.as_ref() {
        Some(path) => Some(config_sources::read_required_config(path)?),
        None => config_sources::read_config(&paths.project_config)?,
    };
    let selected_connections = options
        .connections
        .as_ref()
        .or_else(|| {
            paths
                .project_connections
                .as_ref()
                .filter(|path| path.exists())
        })
        .or_else(|| paths.user_connections.as_ref().filter(|path| path.exists()));
    let connections =
        config_sources::read_connections(selected_connections, options.connections.is_some())?;
    let env_file = match options.env_file.as_ref() {
        Some(path) => config_sources::read_env_file(path)?,
        None => BTreeMap::new(),
    };
    let mut secret_values = env_file.clone();
    secret_values.extend(process.clone());
    let input = ResolutionInput::new(connections.clone())
        .with_user(user.unwrap_or_default())
        .with_project(project.unwrap_or_default())
        .with_env_file(env_file)
        .with_process_env(process)
        .with_cli(CliOverrides {
            profile: options.profile.clone(),
            allow_data_sharing: options.allow_data_sharing.then_some(true),
            ..Default::default()
        });
    Ok(RuntimeConfig {
        resolved: resolve(input)?,
        connections,
        config_path: selected_config.cloned(),
        connections_path: selected_connections.cloned(),
        secret_values,
    })
}

impl std::fmt::Debug for RuntimeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeConfig")
            .field("resolved", &self.resolved)
            .field("connections", &self.connections)
            .field("config_path", &self.config_path)
            .field("connections_path", &self.connections_path)
            .field("secret_values", &"[redacted]")
            .finish()
    }
}

pub fn approval_mode(options: &GlobalOptions) -> Result<saya_agent::ApprovalPolicy, RuntimeError> {
    let value = match options.approval_mode.as_deref() {
        Some(value) => value,
        None if options.non_interactive => "never",
        None => "ask",
    };
    value
        .parse()
        .map_err(|error: saya_agent::ApprovalPolicyParseError| {
            RuntimeError::Approval(error.to_string())
        })
}

pub fn approval_name(options: &GlobalOptions) -> Result<String, RuntimeError> {
    Ok(match approval_mode(options)? {
        saya_agent::ApprovalPolicy::Ask => "ask",
        saya_agent::ApprovalPolicy::ReadOnly => "read-only",
        saya_agent::ApprovalPolicy::Never => "never",
    }
    .into())
}

pub fn format_name(
    options: &GlobalOptions,
    resolved: &saya_config::ResolvedConfig,
) -> crate::render::RenderFormat {
    if options.format != crate::cli::FormatArg::Text {
        options.format.into()
    } else {
        resolved.output_format.into()
    }
}
