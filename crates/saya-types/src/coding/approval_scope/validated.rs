//! Bounded, validated value types carried inside coding approval scopes.

use serde::{Deserialize, Serialize};

use super::ApprovalScopeError;

const MAX_SCOPE_PATH_BYTES: usize = 512;
const MAX_COMMAND_REFERENCE_BYTES: usize = 128;
const MAX_MEMORY_KEY_BYTES: usize = 128;

/// A validated workspace path held inside a write request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspacePath(String);

impl WorkspacePath {
    pub(super) fn new(value: String) -> Result<Self, ApprovalScopeError> {
        validate_relative_path(&value, false)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for WorkspacePath {
    type Error = ApprovalScopeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for WorkspacePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

/// A validated checkout-relative process working directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkingDirectory(String);

impl WorkingDirectory {
    pub(super) fn new(value: String) -> Result<Self, ApprovalScopeError> {
        validate_relative_path(&value, true)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for WorkingDirectory {
    type Error = ApprovalScopeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for WorkingDirectory {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

/// A safe reference naming a configured command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SafeCommandReference(String);

impl SafeCommandReference {
    pub(super) fn new(value: String) -> Result<Self, ApprovalScopeError> {
        if value.is_empty()
            || value.len() > MAX_COMMAND_REFERENCE_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(ApprovalScopeError::InvalidCommandReference);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SafeCommandReference {
    type Error = ApprovalScopeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for SafeCommandReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

/// A bounded key naming a repository memory entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryKey(String);

impl MemoryKey {
    pub(super) fn new(value: String) -> Result<Self, ApprovalScopeError> {
        if value.is_empty()
            || value.len() > MAX_MEMORY_KEY_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ApprovalScopeError::InvalidMemoryKey);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for MemoryKey {
    type Error = ApprovalScopeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for MemoryKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

fn validate_relative_path(value: &str, allow_root: bool) -> Result<(), ApprovalScopeError> {
    if allow_root && value == "." {
        return Ok(());
    }
    if value.is_empty()
        || value.len() > MAX_SCOPE_PATH_BYTES
        || value.starts_with('/')
        || value.as_bytes().get(1) == Some(&b':') && value.as_bytes()[0].is_ascii_alphabetic()
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || value
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(ApprovalScopeError::InvalidPath);
    }
    Ok(())
}
