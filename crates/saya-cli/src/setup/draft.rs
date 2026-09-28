//! The setup draft: what the guided flow collected, and its validation.
//!
//! The API key exists here only as an env-var *name*
//! ([`ProviderDraft::api_key_env`]); the type has no field that could hold a
//! key value, so a value cannot reach the rendered config by construction.

use saya_config::AiProvider;
use saya_types::DatabaseProfile;

use super::SetupError;

pub(crate) const ENV_NAME_MAX: usize = 64;
pub(crate) const PROFILE_NAME_MAX: usize = 64;

/// A setup request: an optional AI provider block and an optional database
/// profile. Both are independent; an empty draft plans nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupDraft {
    pub provider: Option<ProviderDraft>,
    pub profile: Option<ProfileDraft>,
}

/// The `[ai]` section a draft wants. `api_key_env` is an environment-variable
/// name, never a key value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDraft {
    pub provider: AiProvider,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key_env: Option<String>,
}

/// One `[profiles.<name>]` block a draft wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileDraft {
    pub name: String,
    pub profile: DatabaseProfile,
}

impl SetupDraft {
    /// Checks the names a draft carries before anything is read or planned.
    pub fn validate(&self) -> Result<(), SetupError> {
        if let Some(provider) = &self.provider {
            if provider.model.trim().is_empty() {
                return Err(SetupError::Draft("model must not be empty".into()));
            }
            if let Some(env) = &provider.api_key_env {
                validate_env_name(env)?;
            }
        }
        if let Some(profile) = &self.profile {
            validate_profile_name(&profile.name)?;
        }
        Ok(())
    }
}

/// An env-var reference name: `[A-Z_][A-Z0-9_]{0,63}`. Held to this class so
/// the rendered `api_key = { env = "..." }` line never needs TOML escaping.
pub(crate) fn validate_env_name(name: &str) -> Result<(), SetupError> {
    let bytes = name.as_bytes();
    let valid = !bytes.is_empty()
        && bytes.len() <= ENV_NAME_MAX
        && (bytes[0].is_ascii_uppercase() || bytes[0] == b'_')
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_');
    if valid {
        Ok(())
    } else {
        Err(SetupError::Draft(format!(
            "api key environment variable name {name:?} must match [A-Z_][A-Z0-9_]{{0,63}}"
        )))
    }
}

/// A profile name: `[A-Za-z0-9_.-]{1,64}`.
pub(crate) fn validate_profile_name(name: &str) -> Result<(), SetupError> {
    let bytes = name.as_bytes();
    let valid = !bytes.is_empty()
        && bytes.len() <= PROFILE_NAME_MAX
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(SetupError::Draft(format!(
            "profile name {name:?} must match [A-Za-z0-9_.-]{{1,64}}"
        )))
    }
}
