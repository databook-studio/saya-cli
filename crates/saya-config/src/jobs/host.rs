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

/// Rejects a `[run] max_iterations` of zero. The knob no longer feeds any
/// run behaviour — `[jobs] turns` is opt-in and unset means unlimited — so
/// this keeps the stored setting honest: zero would read as a ceiling that
/// pauses everything, a typo, not an intent. There is no upper bound.
pub(crate) fn require_max_iterations(value: usize) -> Result<(), ConfigError> {
    require_at_least_one("max_iterations", value as u64)
}

pub(super) fn require_at_least_one(field: &'static str, value: u64) -> Result<(), ConfigError> {
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

/// Validates the token map against the shape the run contracts carry, so a
/// config the contract would reject at plan-validation time is caught at
/// resolve time instead. The endpoint-count bound is the contract's own
/// (`MAX_BUDGET_ENDPOINTS`), and keys must have the run-scoped name shape.
pub(super) fn resolve_token_map(
    map: &BTreeMap<String, u64>,
) -> Result<BTreeMap<String, u64>, ConfigError> {
    if map.len() > MAX_BUDGET_ENDPOINTS {
        return Err(ConfigError::SettingAboveMaximum {
            field: "tokens_per_endpoint",
            value: map.len(),
            max: MAX_BUDGET_ENDPOINTS,
        });
    }
    for (endpoint, tokens) in map {
        if !is_name_shaped(endpoint) {
            return Err(ConfigError::InvalidEndpointName {
                field: "tokens_per_endpoint",
                key: endpoint.clone(),
            });
        }
        require_at_least_one("tokens_per_endpoint", *tokens)?;
    }
    Ok(map.clone())
}
