use std::collections::BTreeMap;

use crate::ConfigError;

/// Parses content supplied through an explicit `--env-file`; callers opt in to loading it.
pub fn parse_explicit_env_file(content: &str) -> Result<BTreeMap<String, String>, ConfigError> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(parse_line)
        .collect()
}

fn parse_line(line: &str) -> Result<(String, String), ConfigError> {
    let (name, value) = line
        .split_once('=')
        .ok_or_else(|| ConfigError::InvalidEnvironment {
            name: line.into(),
            reason: "expected NAME=value".into(),
        })?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(ConfigError::InvalidEnvironment {
            name: name.into(),
            reason: "invalid environment variable name".into(),
        });
    }
    Ok((name.into(), value.trim_matches(['\'', '"']).into()))
}
