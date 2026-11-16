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

/// A bounded absolute path locator for this host: a POSIX absolute path, and
/// on Windows also the drive, UNC, and verbatim forms Windows APIs and
/// `canonicalize` produce. It conveys no ownership or app-state containment;
/// the runtime verifies those facts at each effect.
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
        || value.chars().any(char::is_control)
    {
        return Err(CheckoutValueError::InvalidManagedCheckoutPath);
    }
    if posix_absolute(value) {
        return Ok(());
    }
    #[cfg(windows)]
    if windows_absolute(value) {
        return Ok(());
    }
    Err(CheckoutValueError::InvalidManagedCheckoutPath)
}

fn posix_absolute(value: &str) -> bool {
    value.starts_with('/')
        && value != "/"
        && !value.ends_with('/')
        && !value.contains('\\')
        && !value[1..]
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
}

/// The absolute spellings Windows hands back: `C:\dir`, the UNC
/// `\\server\share\dir`, and the verbatim `\\?\C:\dir` / `\\?\UNC\…` forms
/// `canonicalize` returns. A relative or drive-relative path, a bare root, and
/// any `.`/`..` or empty component stay refused.
#[cfg(windows)]
fn windows_absolute(value: &str) -> bool {
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        return components_at_least(rest, 3);
    }
    if let Some(rest) = value.strip_prefix(r"\\?\") {
        return drive_rest(rest).is_some_and(|rest| components_at_least(rest, 1));
    }
    if let Some(rest) = value.strip_prefix(r"\\") {
        return components_at_least(rest, 3);
    }
    drive_rest(value).is_some_and(|rest| components_at_least(rest, 1))
}

#[cfg(windows)]
fn drive_rest(value: &str) -> Option<&str> {
    match value.as_bytes() {
        [letter, b':', b'\\', ..] if letter.is_ascii_alphabetic() => Some(&value[3..]),
        _ => None,
    }
}

#[cfg(windows)]
fn components_at_least(rest: &str, at_least: usize) -> bool {
    if rest.is_empty() || rest.ends_with(['\\', '/']) {
        return false;
    }
    let mut count = 0;
    for part in rest.split(['\\', '/']) {
        if part.is_empty() || matches!(part, "." | "..") {
            return false;
        }
        count += 1;
    }
    count >= at_least
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
