use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_MANAGED_CHECKOUT_PATH_BYTES: usize = 4096;

/// The observed lifecycle label for one managed checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ManagedCheckoutLifecycle {
    Creating,
    Active,
    Retained,
    Removing,
    Removed,
    Failed,
    Uncertain,
}

/// A full lowercase Git object name. Its length identifies SHA-1 or SHA-256;
/// repository membership and object type are checked by the Git service.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct GitObjectId(String);

impl GitObjectId {
    pub fn parse(value: &str) -> Result<Self, CheckoutValueError> {
        if !matches!(value.len(), 40 | 64)
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(CheckoutValueError::InvalidGitObjectId);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GitObjectId {
    type Error = CheckoutValueError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

/// A bounded POSIX absolute path locator. It conveys no ownership or
/// app-state containment; the runtime verifies those facts at each effect.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ManagedCheckoutPath(String);

impl ManagedCheckoutPath {
    pub fn new(value: impl Into<String>) -> Result<Self, CheckoutValueError> {
        let value = value.into();
        validate_managed_path(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ManagedCheckoutPath {
    type Error = CheckoutValueError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

fn validate_managed_path(value: &str) -> Result<(), CheckoutValueError> {
    if value.is_empty()
        || value.len() > MAX_MANAGED_CHECKOUT_PATH_BYTES
        || !value.starts_with('/')
        || value == "/"
        || value.ends_with('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || value[1..]
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(CheckoutValueError::InvalidManagedCheckoutPath);
    }
    Ok(())
}

/// Why a checkout identifier or path failed its lexical wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CheckoutValueError {
    #[error("Git object ID must be 40 or 64 lowercase hexadecimal characters")]
    InvalidGitObjectId,
    #[error("managed checkout path is not a bounded POSIX absolute locator")]
    InvalidManagedCheckoutPath,
}
