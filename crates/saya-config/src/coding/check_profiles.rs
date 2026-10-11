use std::{collections::BTreeMap, fmt};

use saya_types::{MAX_RECEIPT_REFERENCE_BYTES, is_bare_name};

use crate::{ConfigError, model::CheckProfileFile};

const MAX_CHECK_PROFILES: usize = 16;
const MAX_CHECK_ARGV_ITEMS: usize = 128;
const MAX_CHECK_ARG_BYTES: usize = 16 * 1024;
const MAX_CHECK_ARGV_BYTES: usize = 128 * 1024;
const MAX_CHECK_TIMEOUT_SECONDS: u64 = 900;

/// One validated configured check profile. Its fields are immutable after
/// resolution; argv is exposed only to the trusted caller that builds C03a's
/// invocation data.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedCheckProfile {
    profile_id: String,
    command_id: String,
    program: String,
    argv: Vec<String>,
    timeout_seconds: u64,
    toolchain: Option<String>,
    environment_profile: String,
}

impl fmt::Debug for ResolvedCheckProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedCheckProfile")
            .field("profile_id", &self.profile_id)
            .field("command_id", &self.command_id)
            .field("program", &self.program)
            .field("argv_count", &self.argv.len())
            .field("timeout_seconds", &self.timeout_seconds)
            .field("toolchain", &self.toolchain)
            .field("environment_profile", &self.environment_profile)
            .finish()
    }
}

impl ResolvedCheckProfile {
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    pub fn command_id(&self) -> &str {
        &self.command_id
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    pub fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds
    }

    pub fn toolchain_reference(&self) -> Option<&str> {
        self.toolchain.as_deref()
    }

    pub fn environment_profile_reference(&self) -> &str {
        &self.environment_profile
    }

    pub(crate) fn argv_count(&self) -> usize {
        self.argv.len()
    }
}

pub(super) fn resolve_check_profiles(
    profiles: Option<&BTreeMap<String, CheckProfileFile>>,
) -> Result<BTreeMap<String, ResolvedCheckProfile>, ConfigError> {
    let Some(profiles) = profiles else {
        return Ok(BTreeMap::new());
    };
    if profiles.len() > MAX_CHECK_PROFILES {
        return Err(ConfigError::TooManyCheckProfiles {
            value: profiles.len(),
            max: MAX_CHECK_PROFILES,
        });
    }

    profiles
        .iter()
        .map(|(profile_id, profile)| {
            validate_profile(profile_id, profile)?;
            Ok((
                profile_id.clone(),
                ResolvedCheckProfile {
                    profile_id: profile_id.clone(),
                    command_id: profile.command_id.clone(),
                    program: profile.program.clone(),
                    argv: profile.argv.clone(),
                    timeout_seconds: profile.timeout_seconds,
                    toolchain: profile.toolchain.clone(),
                    environment_profile: profile.environment_profile.clone(),
                },
            ))
        })
        .collect()
}

fn validate_profile(profile_id: &str, profile: &CheckProfileFile) -> Result<(), ConfigError> {
    if !valid_reference(profile_id) {
        return Err(invalid_profile("profile_id"));
    }
    if !valid_reference(&profile.command_id) {
        return Err(invalid_profile("command_id"));
    }
    if !valid_reference(&profile.program) || !is_bare_name(&profile.program) {
        return Err(invalid_profile("program"));
    }
    if !(1..=MAX_CHECK_TIMEOUT_SECONDS).contains(&profile.timeout_seconds) {
        return Err(invalid_profile("timeout_seconds"));
    }
    if !valid_reference(&profile.environment_profile) {
        return Err(invalid_profile("environment_profile"));
    }
    if profile
        .toolchain
        .as_ref()
        .is_some_and(|reference| !valid_reference(reference))
    {
        return Err(invalid_profile("toolchain"));
    }
    if profile.argv.len() > MAX_CHECK_ARGV_ITEMS {
        return Err(invalid_profile("argv"));
    }
    let mut total = 0usize;
    for argument in &profile.argv {
        if argument.len() > MAX_CHECK_ARG_BYTES || argument.contains('\0') {
            return Err(invalid_profile("argv"));
        }
        total = total.saturating_add(argument.len());
        if total > MAX_CHECK_ARGV_BYTES {
            return Err(invalid_profile("argv"));
        }
    }
    Ok(())
}

fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_RECEIPT_REFERENCE_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn invalid_profile(field: &'static str) -> ConfigError {
    ConfigError::InvalidCheckProfile { field }
}
