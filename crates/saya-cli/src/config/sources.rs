use super::runtime::RuntimeError;
use saya_config::{ConfigFile, ConnectionsFile, MAX_CONFIG_FILE_BYTES, parse_explicit_env_file};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[cfg(test)]
#[path = "sources_tests.rs"]
mod tests;

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
    let value = read_config_text(path).map_err(|source| RuntimeError::Read {
        path: path.clone(),
        source,
    })?;
    Ok(Some(ConfigFile::from_toml(&value)?))
}

pub(crate) fn read_required_config(path: &Path) -> Result<ConfigFile, RuntimeError> {
    if !path.exists() {
        return Err(RuntimeError::Missing(path.into()));
    }
    let value = read_config_text(path).map_err(|source| RuntimeError::Read {
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

/// Both config adapters read the same opened regular file under a byte cap.
/// Config symlinks remain supported; Unix nonblocking open refuses FIFO traps.
fn read_config_text(path: &Path) -> io::Result<String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "configuration must be a regular file",
        ));
    }
    if metadata.len() > MAX_CONFIG_FILE_BYTES as u64 {
        return Err(config_too_large());
    }
    let mut bytes = Vec::with_capacity(MAX_CONFIG_FILE_BYTES + 1);
    file.take((MAX_CONFIG_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CONFIG_FILE_BYTES {
        return Err(config_too_large());
    }
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "configuration must be UTF-8"))
}

fn config_too_large() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("configuration exceeds the {MAX_CONFIG_FILE_BYTES}-byte limit"),
    )
}
