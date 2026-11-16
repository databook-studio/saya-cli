use super::*;

/// Resolves `[host_commands]`: `pass_env` names must be well-formed
/// `NAME=value` names (the child's own rule, checked here so a typo fails
/// at resolve, not at spawn), and `timeout_seconds` follows the
/// one-second-floor discipline. Every key is optional; shaping keys only —
/// neither decides whether the lane composes.
pub(crate) fn resolve_host_commands(
    file: crate::model::HostCommandsFile,
) -> Result<ResolvedHostCommands, ConfigError> {
    for name in &file.pass_env {
        if !is_host_env_name(name) {
            return Err(ConfigError::InvalidHostPassEnv { name: name.clone() });
        }
    }
    let mut pass_env = file.pass_env;
    pass_env.sort();
    pass_env.dedup();
    let timeout_seconds = file
        .timeout_seconds
        .unwrap_or(HOST_COMMANDS_TIMEOUT_SECONDS);
    require_at_least_one("host_commands.timeout_seconds", timeout_seconds)?;
    Ok(ResolvedHostCommands {
        pass_env,
        timeout_seconds,
    })
}

/// True when `name` can appear on the left of `NAME=value`: the child's own
/// rule (see `saya-harness`' host env), checked at resolve so a typo fails
/// with the section's name on it, not at spawn.
fn is_host_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(crate) fn require_at_least_one(field: &'static str, value: u64) -> Result<(), ConfigError> {
    if value >= MIN_BUDGET {
        Ok(())
    } else {
        Err(ConfigError::SettingBelowMinimum {
            field,
            value: value as usize,
            min: MIN_BUDGET as usize,
        })
    }
}
