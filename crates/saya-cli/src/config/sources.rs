use super::runtime::RuntimeError;
use saya_config::{ConfigFile, ConnectionsFile, parse_explicit_env_file};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub(crate) struct Paths {
    pub(crate) user_config: Option<PathBuf>,
    pub(crate) user_connections: Option<PathBuf>,
    pub(crate) project_config: Option<PathBuf>,
    pub(crate) project_connections: Option<PathBuf>,
}

impl Paths {
    pub(crate) fn discover(cwd: &Path, user: &Path) -> Self {
        let project = cwd.join(".saya");
        Self {
            user_config: Some(user.join("config.toml")),
            user_connections: Some(user.join("connections.toml")),
            project_config: Some(project.join("config.toml")),
            project_connections: Some(project.join("connections.toml")),
        }
    }
}

pub(crate) fn user_config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("SAYA_CONFIG_HOME") {
        return PathBuf::from(path).join("saya");
    }
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("saya");
    }
    if let Some(path) = std::env::var_os("APPDATA") {
        return PathBuf::from(path).join("saya");
    }
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".config/saya")
}

pub(crate) fn read_config(path: &Option<PathBuf>) -> Result<Option<ConfigFile>, RuntimeError> {
    let Some(path) = path else { return Ok(None) };
    if !path.exists() {
        return Ok(None);
    }
    let value = fs::read_to_string(path).map_err(|source| RuntimeError::Read {
        path: path.clone(),
        source,
    })?;
    Ok(Some(ConfigFile::from_toml(&value)?))
}

pub(crate) fn read_required_config(path: &Path) -> Result<ConfigFile, RuntimeError> {
    if !path.exists() {
        return Err(RuntimeError::Missing(path.into()));
    }
    let value = fs::read_to_string(path).map_err(|source| RuntimeError::Read {
        path: path.into(),
        source,
    })?;
    Ok(ConfigFile::from_toml(&value)?)
}

pub(crate) fn read_connections(
    path: Option<&PathBuf>,
    explicit: bool,
) -> Result<ConnectionsFile, RuntimeError> {
    let Some(path) = path else {
        return Ok(ConnectionsFile::default());
    };
    if !path.exists() {
        return if explicit {
            Err(RuntimeError::Missing(path.clone()))
        } else {
            Ok(ConnectionsFile::default())
        };
    }
    let value = fs::read_to_string(path).map_err(|source| RuntimeError::Read {
        path: path.clone(),
        source,
    })?;
    Ok(ConnectionsFile::from_toml(&value)?)
}

pub(crate) fn read_env_file(path: &Path) -> Result<BTreeMap<String, String>, RuntimeError> {
    if !path.exists() {
        return Err(RuntimeError::Missing(path.into()));
    }
    let value = fs::read_to_string(path).map_err(|source| RuntimeError::Read {
        path: path.into(),
        source,
    })?;
    Ok(parse_explicit_env_file(&value)?)
}

pub(crate) fn process_env() -> BTreeMap<String, String> {
    std::env::vars().collect()
}
