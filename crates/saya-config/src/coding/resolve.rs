use std::collections::BTreeMap;

use crate::{CodingFile, ConfigError};

use super::check_profiles::{ResolvedCheckProfile, resolve_check_profiles};
use super::limits::{
    MAX_CHILDREN, MAX_CONCURRENT_COMMANDS, MAX_CONCURRENT_MODEL_REQUESTS, MAX_ELAPSED_SECONDS,
    MAX_KNOWN_TOKENS, MAX_LOGICAL_MODEL_REQUESTS, MAX_PHYSICAL_TRANSPORT_ATTEMPTS, MAX_TOOL_CALLS,
    require, require_u64,
};

/// Effective trusted coding limits. These settings shape admission only; they
/// never grant model, filesystem, process, network, or database authority.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCoding {
    pub enabled: bool,
    pub max_children: usize,
    pub max_concurrent_model_requests: usize,
    pub max_concurrent_commands: usize,
    pub max_logical_model_requests: usize,
    pub max_physical_transport_attempts: usize,
    pub max_tool_calls: usize,
    pub max_known_tokens: u64,
    pub max_elapsed_seconds: u64,
    /// Validated fixed check data; it does not grant process or path authority.
    pub check_profiles: BTreeMap<String, ResolvedCheckProfile>,
}

impl Default for ResolvedCoding {
    fn default() -> Self {
        Self {
            enabled: false,
            max_children: MAX_CHILDREN,
            max_concurrent_model_requests: MAX_CONCURRENT_MODEL_REQUESTS,
            max_concurrent_commands: MAX_CONCURRENT_COMMANDS,
            max_logical_model_requests: MAX_LOGICAL_MODEL_REQUESTS,
            max_physical_transport_attempts: MAX_PHYSICAL_TRANSPORT_ATTEMPTS,
            max_tool_calls: MAX_TOOL_CALLS,
            max_known_tokens: MAX_KNOWN_TOKENS,
            max_elapsed_seconds: MAX_ELAPSED_SECONDS,
            check_profiles: BTreeMap::new(),
        }
    }
}

impl ResolvedCoding {
    /// Selects one exact configured profile. Unknown or malformed IDs are not
    /// included in the returned error because callers may pass model text.
    pub fn select_check_profile(
        &self,
        profile_id: &str,
    ) -> Result<&ResolvedCheckProfile, ConfigError> {
        self.check_profiles
            .get(profile_id)
            .filter(|profile| profile.profile_id() == profile_id)
            .ok_or(ConfigError::UnknownCheckProfile)
    }
}

pub(crate) fn resolve(file: &CodingFile) -> Result<ResolvedCoding, ConfigError> {
    let defaults = ResolvedCoding::default();
    let max_children = file.max_children.unwrap_or(defaults.max_children);
    let max_concurrent_model_requests = file
        .max_concurrent_model_requests
        .unwrap_or(defaults.max_concurrent_model_requests);
    let max_concurrent_commands = file
        .max_concurrent_commands
        .unwrap_or(defaults.max_concurrent_commands);
    let max_logical_model_requests = file
        .max_logical_model_requests
        .unwrap_or(defaults.max_logical_model_requests);
    let max_physical_transport_attempts = file
        .max_physical_transport_attempts
        .unwrap_or(defaults.max_physical_transport_attempts);
    let max_tool_calls = file.max_tool_calls.unwrap_or(defaults.max_tool_calls);
    let max_known_tokens = file.max_known_tokens.unwrap_or(defaults.max_known_tokens);
    let max_elapsed_seconds = file
        .max_elapsed_seconds
        .unwrap_or(defaults.max_elapsed_seconds);

    require(max_children, "coding.max_children", MAX_CHILDREN)?;
    require(
        max_concurrent_model_requests,
        "coding.max_concurrent_model_requests",
        MAX_CONCURRENT_MODEL_REQUESTS,
    )?;
    require(
        max_concurrent_commands,
        "coding.max_concurrent_commands",
        MAX_CONCURRENT_COMMANDS,
    )?;
    require(
        max_logical_model_requests,
        "coding.max_logical_model_requests",
        MAX_LOGICAL_MODEL_REQUESTS,
    )?;
    require(
        max_physical_transport_attempts,
        "coding.max_physical_transport_attempts",
        MAX_PHYSICAL_TRANSPORT_ATTEMPTS,
    )?;
    if max_physical_transport_attempts < max_logical_model_requests {
        return Err(ConfigError::SettingOutOfRange {
            field: "coding.max_physical_transport_attempts",
            value: max_physical_transport_attempts,
            min: max_logical_model_requests,
            max: MAX_PHYSICAL_TRANSPORT_ATTEMPTS,
        });
    }
    require(max_tool_calls, "coding.max_tool_calls", MAX_TOOL_CALLS)?;
    require_u64(
        max_known_tokens,
        "coding.max_known_tokens",
        MAX_KNOWN_TOKENS,
    )?;
    require_u64(
        max_elapsed_seconds,
        "coding.max_elapsed_seconds",
        MAX_ELAPSED_SECONDS,
    )?;
    let check_profiles = resolve_check_profiles(file.check_profiles.as_ref())?;

    Ok(ResolvedCoding {
        enabled: file.enabled.unwrap_or(defaults.enabled),
        max_children,
        max_concurrent_model_requests,
        max_concurrent_commands,
        max_logical_model_requests,
        max_physical_transport_attempts,
        max_tool_calls,
        max_known_tokens,
        max_elapsed_seconds,
        check_profiles,
    })
}
